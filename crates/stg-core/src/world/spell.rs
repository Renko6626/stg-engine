//! 符卡世界逻辑：绑定查询、宣言、资格、逐帧计器与结算。
//! 数据槽及常量仍由 `crate::spell` 持有；伤害下钳及相位调用位置仍归 `settle`。

use super::{STATUS_BAD_ARGS, WorldBody};
use crate::enemy::{ENEMY_DYING, EnemyHandle};
use crate::events::Event;
use crate::math::Fx;
use crate::player::LIFE_ALIVE;
use crate::spell::{SPELL_END_HP, SPELL_END_TIMEOUT};

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
    /// 全部 active 符卡槽当场失格（玩法刀）。三个调用点：`try_stop`（冻结期间轮询不跑）、
    /// `try_bomb`（Classic 起爆）、`rewind_landed`（快照带回了被弹前的资格）。
    pub(crate) fn void_spell_captures(&mut self) {
        for s in self.spells.iter_mut() {
            if s.active != 0 {
                s.capture_ok = 0;
            }
        }
    }

    /// settle 符卡趟（相位 7 settle 尾调用，spec §3 五步，按槽升序；`_tables` 现未消费，
    /// 签名与其余相位函数对齐，供将来符卡配置搬进表时零成本扩展）。
    pub(crate) fn settle_spells(&mut self, _tables: &crate::tables::WorldTables) {
        for slot in 0..crate::boss::MAX_BOSSES {
            if self.spells[slot].active == 0 {
                continue;
            }
            // 1. 资格轮询作废（先于一切）：中弹入决死窗 → capture_ok 清 0。另外三处由触发点直接调
            //    `void_spell_captures`，本轮询看不到它们：`try_stop`（冻结期间 settle 不跑）、
            //    `try_bomb`（起爆不改生命态）、`rewind_landed`（快照带回了被弹前的资格）。
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

    /// 找到绑定某敌（索引 + 代）的 active 符卡槽（逃生舱口/读时器/伤害下钳共用，spec
    /// 2026-07-24 §3.1/§4）。按槽升序（I4）；无绑定 None。
    pub(super) fn spell_slot_bound_to(&self, index: u16, generation: u16) -> Option<usize> {
        (0..crate::boss::MAX_BOSSES).find(|&slot| {
            self.spells[slot].active != 0
                && self.spells[slot].boss_index == index
                && self.spells[slot].boss_gen == generation
        })
    }

    /// 符卡宣言世界侧核（`SYS_SPELL_BEGIN` 直通；本刀只经此 API 直测，syscall 绑定见 Task 2）。
    /// P4-b：`slot` 越界 / `time_limit==0` / 该槽已 `active` / `boss` 悬垂（含已死）/
    /// `hp_threshold` 大于当前 hp → no-op + `contract_viol` + `STATUS_BAD_ARGS` + 返回
    /// `false`（宁缺勿哑，零副作用）。成功：写满全字段（复用槽写满纪律）——衰减参数一次
    /// 整除定格 + 记 `hp_start`；`push_event(EVT_SPELL_DECLARED)` + `emit_req(REQ_SPELL_DECLARE)`；
    /// 返回 `true`（syscall 层据此决定是否 spawn 模式子任务）。
    #[allow(clippy::too_many_arguments)] // 符卡宣言的天然参数面（同 create_bullets_batch 先例）
    pub fn spell_begin_internal(
        &mut self,
        slot: usize,
        boss: EnemyHandle,
        spell_id: u16,
        time_limit: u16,
        bonus0: u32,
        flags: u8,
        hp_threshold: i32,
    ) -> bool {
        if slot >= crate::boss::MAX_BOSSES || time_limit == 0 || self.spells[slot].active != 0 {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return false;
        }
        let Some(bi) = self.enemies.get(boss) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return false;
        };
        if hp_threshold > self.enemies.hp[bi] {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return false;
        }
        let nonspell = flags & crate::spell::SPELL_NONSPELL != 0;
        // 非符段 bonus 恒 0（传入值忽略、不计违约，boss 换段刀 spec §3.1）。
        let bonus0 = if nonspell { 0 } else { bonus0 };
        let hp_start = self.enemies.hp[bi];
        let bonus_floor = bonus0 / 10;
        let dec_per_frame = (bonus0 - bonus_floor) / time_limit as u32;
        // 代际戳（ABA 修复，复审 Task 2）：先铸新世代再写槽——`spell_seq` 只增不清、跨槽结算
        // 持久，保证同槽换卡后旧卡残留任务（`Task.spell_epoch` 捕获的是旧值）与新卡 epoch
        // 必不相等，相位 2 调度门禁据此杀掉旧卡残党（见 `ecl::vm::run_tasks`）。
        self.spell_seq[slot] = self.spell_seq[slot].wrapping_add(1);
        self.spells[slot] = crate::spell::SpellSlot {
            active: 1,
            flags,
            capture_ok: 1,
            _pad: 0,
            spell_id,
            boss_index: boss.index,
            boss_gen: boss.generation,
            epoch: self.spell_seq[slot],
            frames_left: time_limit,
            hp_threshold,
            hp_start,
            bonus_now: bonus0,
            bonus_floor,
            dec_per_frame,
        };
        // 非符段不宣言（不发事件、不发请求——壳不弹横幅，boss 换段刀 spec §3.1）。
        if !nonspell {
            self.push_event(Event {
                kind: crate::events::EVT_SPELL_DECLARED,
                a_index: boss.index,
                a_gen: boss.generation,
                x: self.enemies.x[bi],
                y: self.enemies.y[bi],
                data: [spell_id as i32, bonus0 as i32],
            });
            let survival_bit = (flags & crate::spell::SPELL_SURVIVAL != 0) as i32;
            self.emit_req(
                crate::consts::REQ_SPELL_DECLARE,
                [
                    spell_id as i32,
                    bonus0 as i32,
                    time_limit as i32,
                    survival_bit,
                    0,
                    0,
                ],
            );
        }
        true
    }

    /// 符卡/非符段结算原子包（spec 2026-07-24 §4 结束矩阵 + boss 换段刀 spec §3.2）。`cause` = `SPELL_END_*`。
    /// 顺序：① 超时钉血 → ② 付分与事件（普通卡按 captured 规则；非符段只发 `EVT_PHASE_ENDED`）
    /// → ③ 自动清弹（除非 `SPELL_NO_CLEAR`；超时路径不给星）→ ④ 写 `spell_last_result`
    /// → ⑤ 槽全字段清零 + `boss_ui` 同步清（复用槽写满纪律的另一半：清空亦是全字段覆写）。
    pub(crate) fn settle_one_spell(&mut self, slot: usize, cause: u8) {
        use crate::spell::{
            SPELL_END_TIMEOUT, SPELL_FAIL_CAPTURE_LOST, SPELL_FAIL_TIMEOUT, SPELL_NO_CLEAR,
            SPELL_NONSPELL, SPELL_SURVIVAL,
        };
        let s = self.spells[slot];
        let boss = EnemyHandle {
            index: s.boss_index,
            generation: s.boss_gen,
        };
        // ① 超时钉血：绑定 boss 仍在且未在死 → hp = min(hp, 血线)，剩血不漏进下一段。
        //    `min` 不是赋值：永远不抬血（判别腿 `hp_break_does_not_raise_hp_to_threshold`）。
        if cause == SPELL_END_TIMEOUT
            && let Some(i) = self.enemies.get(boss)
            && self.enemies.flags[i] & crate::enemy::ENEMY_DYING == 0
        {
            self.enemies.hp[i] = self.enemies.hp[i].min(s.hp_threshold);
        }
        let (x, y) = match self.enemies.get(boss) {
            Some(i) => (self.enemies.x[i], self.enemies.y[i]),
            None => (Fx::ZERO, Fx::ZERO),
        };
        if s.flags & SPELL_NONSPELL != 0 {
            // ② 非符段：不付分、不发 CAPTURED/FAILED/REQ_SPELL_RESULT。
            self.push_event(Event {
                kind: crate::events::EVT_PHASE_ENDED,
                a_index: s.boss_index,
                a_gen: s.boss_gen,
                x,
                y,
                data: [s.spell_id as i32, cause as i32],
            });
        } else {
            // ② 普通卡：普通卡超时恒 FAILED(超时)，其余（HP / 耐久卡超时 / 手动）看资格。
            let (captured, reason) = if cause == SPELL_END_TIMEOUT && s.flags & SPELL_SURVIVAL == 0
            {
                (false, SPELL_FAIL_TIMEOUT)
            } else if s.capture_ok != 0 {
                (true, 0)
            } else {
                (false, SPELL_FAIL_CAPTURE_LOST)
            };
            let paid = if captured { s.bonus_now } else { 0 };
            if captured {
                self.players[0].score += paid as u64;
            }
            let kind = if captured {
                crate::events::EVT_SPELL_CAPTURED
            } else {
                crate::events::EVT_SPELL_FAILED
            };
            self.push_event(Event {
                kind,
                a_index: s.boss_index,
                a_gen: s.boss_gen,
                x,
                y,
                data: [
                    s.spell_id as i32,
                    if captured { paid as i32 } else { reason },
                ],
            });
            self.emit_req(
                crate::consts::REQ_SPELL_RESULT,
                [
                    s.spell_id as i32,
                    captured as i32,
                    paid as i32,
                    reason,
                    0,
                    0,
                ],
            );
        }
        // ③ 自动清弹：超时路径不给星（对齐 ZUN 超时走 etClear）。
        if s.flags & SPELL_NO_CLEAR == 0 {
            let field = if cause == SPELL_END_TIMEOUT {
                crate::field::fullscreen_clear_field_no_star()
            } else {
                crate::field::fullscreen_clear_field()
            };
            self.create_field(field);
        }
        // ④ 结束方式读口（`spell_result` syscall 131）。
        self.spell_last_result[slot] = cause;
        // ⑤ 槽清零。
        self.spells[slot] = crate::spell::SpellSlot::default();
        // B16② 清扫(2026-07-25):结算原子同步清公告板——否则 spells 清零后整槽被
        // settle_spells 首行跳过,boss_ui 冻结旧值(无后续卡时无界陈旧)。次帧若新卡
        // 开(相2),相7 步骤5 自动喂真值,清→开→喂按相位全序串行,无竞态。
        self.boss_ui[slot] = crate::boss::BossUiSlot::default();
    }

    /// 逃生舱口（`SYS_SPELL_END` 世界侧核，自定义结束条件用）：owner 绑定的 active 槽走
    /// HP 路径结算；无绑定 → no-op（**不计** contract——重复调用安全，同 `spell_end` 语义）。
    pub fn spell_end_by_owner(&mut self, boss: EnemyHandle) {
        let Some(slot) = self.spell_slot_bound_to(boss.index, boss.generation) else {
            return;
        };
        self.settle_one_spell(slot, crate::spell::SPELL_END_MANUAL);
    }

    /// 读族（`SYS_SPELL_TIMER` 世界侧核）：owner 绑定的 active 槽返回 `frames_left`；
    /// 无绑定 → `-1`（`wait_spell()` 语法糖的判据）。
    pub fn spell_frames_left_of(&self, boss: EnemyHandle) -> i32 {
        match self.spell_slot_bound_to(boss.index, boss.generation) {
            Some(slot) => self.spells[slot].frames_left as i32,
            None => -1,
        }
    }
}

#[cfg(test)]
mod tests;
