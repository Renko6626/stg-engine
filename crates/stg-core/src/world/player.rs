//! 相位 1 · 输入译码 + 相位 3 · 自机更新（D6/A8）。
//!
//! **命名**：`crate::player` 是 `PlayerState` 数据模块（`bullets`/`enemy`/`field`/`shots` 的同辈）；
//! 本模块是自机的**相位逻辑**。二者并存，路径区分。
//!
//! **生死状态机的触发与计时分家**：本相位（3）独占**全部计时**（决死窗口倒数 → `commit_death`
//! → 按 Kit：原地继续 + 遡行请求 / 场底重生 / GAMEOVER）；**中弹触发**（Alive → DeathWindow）在 settle（相位 7）。
//! 因相位 3 早于 7，中弹在帧尾定、窗口从次帧起数 —— 这 1 帧错位正是决死窗口的语义。

use super::WorldBody;
use crate::events::Event;
use crate::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SLOW, BTN_UP};
use crate::math::Fx;
use crate::player::{LIFE_ABSENT, LIFE_ALIVE, LIFE_DEATHWINDOW, LIFE_GAMEOVER, LIFE_JUMPING};
use crate::shots::ShotInit;
use crate::tables::{BombCfg, BombOrigin, Kit, WorldTables};

impl WorldBody {
    /// **沿检测的滚存点**：`prev_input` 必须在这里、在覆写 `input` 之前滚存旧值——本相位
    /// （1）无论哪个冻结组开着都**照跑**（不受 A/B/C 任何门禁影响），所以即便自机被 ECL
    /// 演出定住（A 组冻），输入记录仍然诚实：`prev_input` 永远是"上一次 `decode_input`
    /// 跑过的电平"，不会被冻结跳过而漂移——`pressed_edge` 才能在解冻的第一帧照样判对。
    pub(crate) fn decode_input(&mut self, input: &crate::input::InputFrame) {
        self.phase_enter(super::PH_DECODE);
        for i in 0..crate::MAX_PLAYERS {
            self.players[i].prev_input = self.players[i].input;
            self.players[i].input = input.actions[i].buttons;
        }
    }

    /// 上升沿判定：本帧该位为 1 且上一帧为 0。**`EDGE_MASK` 声明的沿语义在这里第一次被
    /// 真正消费**——词表译码（`decode_input`）本身只搬电平，不做沿处理，沿检测靠比较
    /// `input`/`prev_input` 两帧快照实现（P6：两者都随快照回滚，重放沿检测逐位可复现）。
    pub(crate) fn pressed_edge(&self, i: usize, btn: u32) -> bool {
        self.players[i].input & btn != 0 && self.players[i].prev_input & btn == 0
    }
    /// 本相位横跨**两个冻结组**（时停刀 spec §3），故循环体按 A/C 切成两段：
    /// 生死状态机计时是 **C 组**（世界对自机的裁决），移动/发弹是 **A 组**（自机的主动行为）。
    ///
    /// 计时归 C 而非 A 是有理由的——它不是自机的行动。放 A 会造出"演出定住你、你中弹进
    /// 决死窗口而窗口计时被冻、又不能 bomb ⇒ 永远挂在决死窗里"的怪状态（spec §3）。
    ///
    /// **未冻时逐位等价于拆分前**：`LIFE_ABSENT | LIFE_GAMEOVER => continue` 从 `match` 的
    /// 一个臂提到循环开头（那两态原本就直接 `continue`，别的臂一个不碰）；`commit_death`
    /// 之后的 GAMEOVER 复查**原样保留在 C 段末**，否则刚耗尽的自机会在同一帧继续动。
    pub(crate) fn update_players(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_PLAYERS);
        let scene = self.scene_frozen();
        let actor = self.actor_frozen();
        for i in 0..crate::MAX_PLAYERS {
            if self.players[i].life_state == LIFE_GAMEOVER {
                // 续关是 GAMEOVER 态唯一响应的输入（壳子刀）：不受 A/C 冻结组门禁——它是
                // 局面级的元操作，不是自机的行动；成功后本帧余下相位按 ALIVE 走（玩法刀：原地复活）。
                self.try_continue(i);
                continue;
            }
            if self.players[i].life_state == LIFE_ABSENT {
                continue;
            }
            // ── C 组：生死状态机计时（A4 相位 3 职责）
            if !scene {
                // 跳躍冷却（玩法刀）：先减后判——落地帧的写满在下面 JUMPING 臂，本帧不被自己减掉。
                if self.players[i].jump_cd > 0 {
                    self.players[i].jump_cd -= 1;
                }
                // 经典 bomb 计时（经典机体刀）：C 组、任何 life_state 下都数；触发帧的写入在 A 组，
                // 故不被自己减掉。
                if self.players[i].bomb_timer > 0 {
                    self.players[i].bomb_timer -= 1;
                }
                match self.players[i].life_state {
                    LIFE_DEATHWINDOW => {
                        // deathstop 挂点：窗口内按 X 会在 A 组 `try_stop` 里拨回 LIFE_ALIVE 并清
                        // state_timer；这里只处理"没人救"的分支。C 组先于 A 组 ⇒ 耗尽那一帧按 X
                        // 已来不及（有效窗口 = 进窗后 DEATHBOMB_WINDOW−1 帧）。
                        if self.players[i].state_timer > 0 {
                            self.players[i].state_timer -= 1;
                        }
                        if self.players[i].state_timer == 0 {
                            self.commit_death(i, tables);
                            // 死亡帧 A 组整段不跑（玩法刀复审）：`commit_death` 已原地复活，否则同帧
                            // 按 X 会在扣命之后再白扣一发停止（无 timeline 宿主可见）。
                            continue;
                        }
                    }
                    LIFE_JUMPING => {
                        // 跳躍倒计时（时间机制内核刀）：恰好 `JUMP_FRAMES` 帧后回 ALIVE。
                        // 计时归 C 组：场景冻结时不走——但 `try_jump` 门禁本就拒绝在冻结中
                        // 起跳，这条只在"跳躍中被 ECL 演出定住"时才有意义。
                        if self.players[i].state_timer > 0 {
                            self.players[i].state_timer -= 1;
                        }
                        if self.players[i].state_timer == 0 {
                            self.players[i].life_state = LIFE_ALIVE;
                            self.players[i].jump_cd = crate::player::JUMP_COOLDOWN;
                        }
                    }
                    LIFE_ALIVE => {
                        if self.players[i].invuln > 0 {
                            self.players[i].invuln -= 1; // 遡行落地 / 续关无敌
                        }
                    }
                    _ => {}
                }
                // commit_death 可能刚把 lives 耗尽置 GAMEOVER → 再判一次跳过移动/发弹
                if self.players[i].life_state == LIFE_GAMEOVER {
                    continue;
                }
            }
            // ── A 组：自机的主动行为（移动 / 发弹 / 用能力）
            if actor {
                continue;
            }
            // 跳躍中 = 缺席（spec §2.2）：A 组整段跳过。放在 `try_jump` **之前**判，
            // 起跳帧本身也不再移动/发弹——影子世界与真跳走同一条路，起跳帧的处置必须唯一。
            if self.players[i].life_state == LIFE_JUMPING {
                continue;
            }
            // 规则套件分派（经典机体刀）：X/C 键行为按机体数据走，穷尽 match。
            match &tables.characters[self.players[i].character_id as usize].kit {
                Kit::Chronos => {
                    self.try_jump(i);
                    if self.players[i].life_state == LIFE_JUMPING {
                        continue;
                    }
                    self.try_stop(i);
                }
                Kit::Classic(bomb) => self.try_bomb(i, bomb),
            }
            self.move_player(i, tables);
            // 角色模块静态分发点（A8"shottype 类似物"）：机体 1 复用机体 0 的 shottype 表
            // （经典机体刀 spec §2.4），将来各角色再各自一臂。
            match self.players[i].character_id {
                0 | 1 => self.char0_update_shot(i, tables),
                _ => {}
            }
        }
    }

    /// 决死窗口耗尽。共用：扣残机 + 偏差值 + `EVT_PLAYER_DIED`；残机耗尽 → GAMEOVER。
    /// 之后按规则套件分派（经典机体刀）：
    /// - `Chronos`（玩法刀 spec §4.2，死亡即遡行）：**原地**回 ALIVE、`REWIND_INVULN`，发
    ///   `EVT_REWIND_REQUESTED`。有 timeline 的宿主据此恢复被弹前的快照，代价在 `rewind_landed`
    ///   从快照重算；无 timeline 的宿主到此为止 = 原地继续。
    /// - `Classic`（东方原作语义）：瞬移场底 `(0, 384)`、ALIVE、`RESPAWN_INVULN`，**不发**请求——
    ///   挂 timeline 也不会遡行。不新增生命态（旧 `LIFE_RESPAWNING` 期间 A 组本就照跑，与 ALIVE+无敌同）。
    fn commit_death(&mut self, i: usize, tables: &WorldTables) {
        let p = &mut self.players[i];
        p.lives = p.lives.saturating_sub(1);
        p.deaths = p.deaths.saturating_add(1);
        let (x, y, lives, hit_frame) = (p.x, p.y, p.lives, p.hit_frame);
        self.push_event(Event {
            kind: crate::events::EVT_PLAYER_DIED,
            a_index: i as u16,
            a_gen: 0,
            x,
            y,
            data: [lives as i32, 0],
        });
        if lives == 0 {
            self.players[i].life_state = LIFE_GAMEOVER;
            return;
        }
        let p = &mut self.players[i];
        p.life_state = LIFE_ALIVE;
        p.state_timer = 0;
        let cid = p.character_id as usize;
        // 各臂内重新取 `self.players[i]`：Chronos 臂要调 `self.push_event`，不能跨调用持有 `p`。
        match &tables.characters[cid].kit {
            Kit::Chronos => {
                let p = &mut self.players[i];
                p.invuln = p.invuln.max(crate::player::REWIND_INVULN);
                self.push_event(Event {
                    kind: crate::events::EVT_REWIND_REQUESTED,
                    a_index: i as u16,
                    a_gen: 0,
                    x,
                    y,
                    data: [hit_frame as i32, 0],
                });
            }
            Kit::Classic(_) => {
                let p = &mut self.players[i];
                p.x = Fx::ZERO;
                p.y = Fx::from_int(384);
                p.invuln = p.invuln.max(crate::player::RESPAWN_INVULN);
            }
        }
    }

    /// 停止触发（A 组，玩法刀 spec §2.2）：时停 + 触碰消弹合一，库存 = `bombs`。门禁四条：
    /// 上升沿（`pressed_edge`，按住跨窗口不连发）+ 库存 > 0 + 未在停止中（停止中再按 no-op
    /// 不扣）+ ALIVE 或决死窗口（deathstop）。deathstop 不退款：进窗口时命没扣，扣命只在
    /// `commit_death`。符卡资格在**触发点**作废——冻结期间 settle 不跑，轮询看不到。
    fn try_stop(&mut self, i: usize) {
        if !self.pressed_edge(i, crate::input::BTN_BOMB)
            || self.players[i].bombs == 0
            || self.freeze_left[0] != 0
            || !matches!(self.players[i].life_state, LIFE_ALIVE | LIFE_DEATHWINDOW)
        {
            return;
        }
        self.players[i].bombs -= 1;
        self.freeze_left[0] = crate::player::TIMESTOP_FRAMES;
        if self.players[i].life_state == LIFE_DEATHWINDOW {
            self.players[i].life_state = LIFE_ALIVE;
            self.players[i].state_timer = 0;
        }
        self.void_spell_captures();
    }

    /// 经典 bomb（`Kit::Classic`，经典机体刀 spec §3.1）。门禁：上升沿、库存 > 0、未在 bomb 中、
    /// ALIVE 或决死窗口（deathbomb；命没扣，无退款）。效果顺序固定：扣库存 → 计时 → 救窗口
    /// → 无敌 → 按声明序铺 field（I4）→ 吸道具（此时已 ALIVE）→ 触发点失格（与 `try_stop` 同口径）。
    /// `cfg` 借自 `tables`（与 `self` 不同对象）；按索引取 field 免得迭代器横跨 `create_field`。
    fn try_bomb(&mut self, i: usize, cfg: &BombCfg) {
        if !self.pressed_edge(i, crate::input::BTN_BOMB)
            || self.players[i].bombs == 0
            || self.players[i].bomb_timer != 0
            || !matches!(self.players[i].life_state, LIFE_ALIVE | LIFE_DEATHWINDOW)
        {
            return;
        }
        let p = &mut self.players[i];
        p.bombs -= 1;
        p.bomb_timer = cfg.frames;
        if p.life_state == LIFE_DEATHWINDOW {
            p.life_state = LIFE_ALIVE;
            p.state_timer = 0;
        }
        p.invuln = cfg.invuln;
        let (px, py) = (p.x, p.y);
        for k in 0..cfg.fields.len() {
            let f = cfg.fields[k];
            let (x, y) = match f.origin {
                BombOrigin::FieldCenter => (Fx::ZERO, Fx::from_int(super::FIELD_HEIGHT / 2)),
                BombOrigin::PlayerAtCast => (px, py),
            };
            self.create_field(crate::field::FieldInit {
                x,
                y,
                radius: f.radius,
                dmg_per_frame: f.dmg_per_frame,
                life: f.life,
                owner: i as u8,
                flags: f.flags,
            });
        }
        if cfg.attract_items {
            self.attract_all_items(i);
        }
        self.void_spell_captures();
    }

    /// 续关（壳子刀 spec §3）：`LIFE_GAMEOVER` 下响应上升沿——残机 / 停止库存回
    /// `Loadout::default()`，`score = continues + 1`（东方惯例：分数变成续关计数），
    /// `continues` 饱和加一，原地复活 + `RESPAWN_INVULN`（玩法刀：场底重生退役）；`deaths` 不清。
    /// power 不动。非 GAMEOVER 下按沿 = no-op（`update_players` 只在 GAMEOVER 臂调本函数）。
    fn try_continue(&mut self, i: usize) {
        if !self.pressed_edge(i, crate::input::BTN_CONTINUE) {
            return;
        }
        let ld = crate::player::Loadout::default();
        let p = &mut self.players[i];
        p.continues = p.continues.saturating_add(1);
        p.lives = ld.lives;
        p.bombs = ld.bombs;
        p.score = p.continues as u64;
        p.life_state = LIFE_ALIVE;
        p.invuln = crate::player::RESPAWN_INVULN;
        p.state_timer = 0;
    }

    /// 跳躍触发（A 组，时间机制内核刀 spec §2.2）。门禁四条：上升沿 + `LIFE_ALIVE` +
    /// 冷却已尽（玩法刀）+ 场景未冻结（时停中按跳躍无效——两种时间能力不叠加，规则只有一条）。
    /// DEATHWINDOW / JUMPING 下按下 = no-op，**不计违约**（玩家操作不是脚本坏参）。
    /// 进入 `LIFE_JUMPING`，`state_timer = JUMP_FRAMES`，倒计时归 C 组。
    fn try_jump(&mut self, i: usize) {
        if !self.pressed_edge(i, crate::input::BTN_JUMP)
            || self.players[i].life_state != LIFE_ALIVE
            || self.scene_frozen()
            || self.players[i].jump_cd != 0
        {
            return;
        }
        self.players[i].life_state = LIFE_JUMPING;
        self.players[i].state_timer = crate::player::JUMP_FRAMES;
    }

    /// 遡行落地（写 API，玩法刀 spec §4.3）：timeline 把世界恢复到被弹前的快照后调它。快照里的
    /// 残机/偏差值/符卡资格都是旧值，**代价全部在这里重算**：偏差值 +1、残机 −1 且下限 1（致死
    /// 与否只在死的那一刻由 `commit_death` 判，落地不再判死）、无敌取 max、active 卡失格。
    /// 落点不一定是 ALIVE（可能正在跳躍），故不断言状态。`i` 越界 → no-op + `contract_viol`（P4-b）。
    pub fn rewind_landed(&mut self, i: usize) {
        if i >= crate::MAX_PLAYERS {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            return;
        }
        let p = &mut self.players[i];
        p.deaths = p.deaths.saturating_add(1);
        p.lives = p.lives.saturating_sub(1).max(1);
        p.invuln = p.invuln.max(crate::player::REWIND_INVULN);
        if p.life_state == LIFE_DEATHWINDOW {
            // 落点快照在决死窗口里（关底 `seal_history` / 窗口内读档后环首帧）：这次死亡的代价
            // 刚付过，拨回 ALIVE——否则窗口在恢复出的世界里再耗尽一次，连环遡行直到 GAMEOVER。
            p.life_state = LIFE_ALIVE;
            p.state_timer = 0;
        }
        self.void_spell_captures();
    }

    /// 移动（东方手感：方向 + 低速 + 对角归一 + 场界钳制）。移速三值读角色配置表
    /// （M0-17 T3：`tables.characters[character_id]`，迁自 player.rs 原 HIGH_SPEED/LOW_SPEED/
    /// INV_SQRT2 常量，零行为搬家）。
    fn move_player(&mut self, i: usize, tables: &WorldTables) {
        let inp = self.players[i].input;
        let mut dx = 0i32;
        let mut dy = 0i32;
        if inp & BTN_LEFT != 0 {
            dx -= 1;
        }
        if inp & BTN_RIGHT != 0 {
            dx += 1;
        }
        if inp & BTN_UP != 0 {
            dy -= 1; // y 向下为正，UP = 减 y
        }
        if inp & BTN_DOWN != 0 {
            dy += 1;
        }
        let cfg = &tables.characters[self.players[i].character_id as usize];
        let sp = if inp & BTN_SLOW != 0 {
            cfg.low_speed
        } else {
            cfg.high_speed
        };
        let axis = if dx != 0 && dy != 0 {
            sp * cfg.inv_sqrt2
        } else {
            sp
        }; // 对角归一
        let p = &mut self.players[i];
        if dx > 0 {
            p.x = p.x + axis;
        } else if dx < 0 {
            p.x = p.x - axis;
        }
        if dy > 0 {
            p.y = p.y + axis;
        } else if dy < 0 {
            p.y = p.y - axis;
        }
        // 场界钳制（自机不出场）
        p.x = Fx::from_raw(p.x.raw().clamp(
            Fx::from_int(-super::FIELD_HALF_W).raw(),
            Fx::from_int(super::FIELD_HALF_W).raw(),
        ));
        p.y = Fx::from_raw(p.y.raw().clamp(0, Fx::from_int(super::FIELD_HEIGHT).raw()));
    }

    /// character-0 火力：相位 3 shottype 表解释器（M0-17 T4 通电；spec「相位 3 解释器」节）。
    ///
    /// **计时器语义（钉死）**：SHOT 松开 → `shot_timer` 清零；持住 → 用**自增前**的当前值
    /// 判 `shot_timer % interval == delay % interval`（先判后加），随后 `wrapping_add(1)`。
    /// 选"先判后加"而非"先加后判"是为了让**首次持住的那一帧**（`shot_timer == 0`）能在
    /// `delay == 0` 时立即命中（v0 全表 `delay = 0`）——与旧 `shot_cd` 倒计时的直觉一致
    /// （`player_shot_fires_on_button` 沿用旧断言：按下当帧就出弹），并被
    /// `shot_timer_phase_and_release_reset` 判别测试钉死（松 1 帧再持，首发延迟须与初次
    /// 一致——若改成"先加后判"，首发会晚 `interval` 帧才出现，测试会红）。
    fn char0_update_shot(&mut self, i: usize, tables: &WorldTables) {
        if self.players[i].input & crate::input::BTN_SHOT == 0 {
            self.players[i].shot_timer = 0;
            return;
        }
        let timer = self.players[i].shot_timer; // 自增前的值——发射判定用它
        self.players[i].shot_timer = timer.wrapping_add(1);

        let cfg = &tables.characters[self.players[i].character_id as usize];
        let tier = self.players[i].power_tier() as usize;
        let focus = if self.players[i].input & BTN_SLOW != 0 {
            1
        } else {
            0
        };
        let shooters = &cfg.shot.sets[tier][focus];
        let option_pos = &cfg.shot.option_pos[tier];
        let (px, py) = (self.players[i].x, self.players[i].y);

        for shooter in shooters.iter() {
            debug_assert!(
                shooter.interval != 0,
                "interval==0 应被加载期 validate() 挡下（外部表兜底）"
            );
            if timer % shooter.interval == shooter.delay % shooter.interval {
                let (ox, oy) = if shooter.option == 0 {
                    (shooter.dx, shooter.dy)
                } else {
                    let (opx, opy) = option_pos[(shooter.option - 1) as usize];
                    (opx + shooter.dx, opy + shooter.dy)
                };
                let (vx, vy) = crate::math::polar_to_vec(shooter.speed, shooter.angle);
                self.create_player_shot(ShotInit {
                    x: px + ox,
                    y: py + oy,
                    vx,
                    vy,
                    damage: shooter.damage,
                    radius: shooter.radius,
                    sprite: shooter.sprite,
                    owner: i as u8,
                    flags: 0,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::input::{BTN_BOMB, BTN_JUMP, InputFrame};
    use crate::player::{JUMP_FRAMES, LIFE_JUMPING, REWIND_INVULN};
    use crate::world::test_support::{bullet_at, step_t};

    fn keys(buttons: u32) -> InputFrame {
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = buttons;
        f
    }

    // ── 时间机制内核刀（2026-09-07）：跳躍 / 遡行请求 / 落地 ───────────────────

    /// 跳躍进入 JUMPING 且计时恰为 `JUMP_FRAMES`；恰好 N 帧后回 ALIVE（N−1 帧仍在跳，判别
    /// N±1）。
    #[test]
    fn jump_enters_jumping_and_returns_alive_after_exactly_n_frames() {
        use crate::player::LIFE_ALIVE;
        let mut w = crate::step::World::new(1);
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_JUMPING);
        assert_eq!(w.body.players[0].state_timer, JUMP_FRAMES);
        for _ in 0..(JUMP_FRAMES - 1) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(
            w.body.players[0].life_state, LIFE_JUMPING,
            "第 N−1 帧仍在跳"
        );
        step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(
            w.body.players[0].life_state, LIFE_ALIVE,
            "恰第 N 帧回 ALIVE"
        );
    }

    /// 冷却判别（落地后，玩法刀）：再走 598 帧（cd=2）按 JUMP 仍 ALIVE；再走 599 帧（cd=1）按
    /// JUMP——本帧 C 组先减到 0、A 组门禁放行 ⇒ 起跳。两腿夹住「恰好 600 帧」。
    #[test]
    fn jump_cooldown_blocks_until_exactly_expired() {
        use crate::player::{JUMP_COOLDOWN, LIFE_ALIVE};
        let land = || {
            let mut w = crate::step::World::new(1);
            step_t(&mut w, &keys(BTN_JUMP));
            for _ in 0..JUMP_FRAMES {
                step_t(&mut w, &InputFrame::empty(0));
            }
            assert_eq!(w.body.players[0].life_state, LIFE_ALIVE);
            assert_eq!(w.body.players[0].jump_cd, JUMP_COOLDOWN, "落地那帧写满冷却");
            w
        };
        let mut w = land();
        for _ in 0..(JUMP_COOLDOWN - 2) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE, "cd 未尽不得跳");
        let mut w = land();
        for _ in 0..(JUMP_COOLDOWN - 1) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_JUMPING, "cd 恰尽即可跳");
    }

    /// 冷却归 C 组：停止冻结期间不走。
    #[test]
    fn jump_cooldown_does_not_tick_while_scene_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].jump_cd = 100;
        w.body.freeze_left = [11, 0];
        for _ in 0..10 {
            step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].jump_cd, 100);
    }

    #[test]
    fn jump_cd_enters_the_checksum() {
        let mut w = crate::step::World::new(1);
        let c0 = w.checksum();
        w.body.players[0].jump_cd = 1;
        assert_ne!(w.checksum(), c0);
    }

    /// 门禁：DEATHWINDOW / 场景冻结 / 跳躍中再按 → 零变化。
    #[test]
    fn jump_gate_rejects_non_alive_frozen_and_rejump() {
        use crate::player::{DEATHBOMB_WINDOW, LIFE_DEATHWINDOW};
        // DEATHWINDOW
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = DEATHBOMB_WINDOW;
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_DEATHWINDOW);
        // 场景冻结（玩家时停中）
        let mut w = crate::step::World::new(1);
        w.body.freeze_left[0] = 10;
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
        // 跳躍中松开再按：计时不得重置
        let mut w = crate::step::World::new(1);
        step_t(&mut w, &keys(BTN_JUMP));
        step_t(&mut w, &InputFrame::empty(0));
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_JUMPING);
        assert_eq!(
            w.body.players[0].state_timer,
            JUMP_FRAMES - 2,
            "再按不重置计时"
        );
    }

    /// 缺席语义①②：弹压在自机上，ALIVE 对照组进决死窗口，JUMPING 不中弹也不擦弹。
    #[test]
    fn jumping_player_is_neither_hit_nor_grazed() {
        use crate::player::LIFE_DEATHWINDOW;
        let mut ctl = crate::step::World::new(1);
        bullet_at(&mut ctl, 0, 384);
        step_t(&mut ctl, &InputFrame::empty(0));
        assert_eq!(
            ctl.body.players[0].life_state, LIFE_DEATHWINDOW,
            "对照组必须中弹"
        );
        assert_eq!(ctl.body.players[0].graze, 1, "对照组必须擦到");

        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = LIFE_JUMPING;
        w.body.players[0].state_timer = JUMP_FRAMES;
        bullet_at(&mut w, 0, 384);
        step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.players[0].life_state, LIFE_JUMPING);
        assert_eq!(w.body.players[0].graze, 0, "缺席不擦弹");
    }

    /// 缺席语义③④⑤：按方向不动、按射击不出弹、道具贴身不拾取（ALIVE 对照组三者皆发生）。
    #[test]
    fn jumping_player_does_not_move_shoot_or_pick() {
        use crate::input::{BTN_RIGHT, BTN_SHOT};
        use crate::math::Fx;
        let run = |jumping: bool| {
            let mut w = crate::step::World::new(1);
            if jumping {
                w.body.players[0].life_state = LIFE_JUMPING;
                w.body.players[0].state_timer = JUMP_FRAMES;
            }
            w.body.drop_item(
                Fx::ZERO,
                Fx::from_int(384),
                crate::items::ITEM_POINT,
                &crate::tables::TABLES_V0,
            );
            let x0 = w.body.players[0].x;
            for _ in 0..3 {
                step_t(&mut w, &keys(BTN_RIGHT | BTN_SHOT));
            }
            (
                w.body.players[0].x != x0,
                w.body.shots.iter_alive().count() > 0,
                w.body.items.iter_alive().count() == 0,
            )
        };
        assert_eq!(run(false), (true, true, true), "对照组：动了/射了/吃了");
        assert_eq!(run(true), (false, false, false), "缺席：不动/不射/不吃");
    }

    /// 续关判别：GAMEOVER 按沿 → 残机/雷/时停回默认、分数 = 续关数、计数 +1、重生无敌；
    /// ALIVE 按沿零变化；饱和不回绕。
    #[test]
    fn continue_restores_defaults_from_gameover_and_is_noop_otherwise() {
        use crate::input::BTN_CONTINUE;
        use crate::math::Fx;
        use crate::player::{LIFE_ALIVE, LIFE_GAMEOVER, Loadout, RESPAWN_INVULN};
        let ld = Loadout::default();
        // 对照：ALIVE 下按沿零变化
        let mut w = crate::step::World::new(1);
        w.body.players[0].score = 12345;
        let c0 = w.checksum();
        step_t(&mut w, &keys(BTN_CONTINUE));
        let mut probe = crate::step::World::new(0);
        w.copy_into(&mut probe);
        probe.body.players[0].input = 0;
        probe.body.players[0].prev_input = 0;
        probe.body.frame = 0;
        assert_eq!(w.body.players[0].continues, 0, "ALIVE 下不续关");
        assert_eq!(w.body.players[0].score, 12345);
        let _ = c0;
        // GAMEOVER 下按沿续关
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = LIFE_GAMEOVER;
        w.body.players[0].lives = 0;
        w.body.players[0].bombs = 0;
        w.body.players[0].score = 999_999;
        w.body.players[0].power = 250;
        w.body.players[0].x = Fx::from_int(-40);
        w.body.players[0].deaths = 3;
        step_t(&mut w, &keys(BTN_CONTINUE));
        let p = &w.body.players[0];
        assert_eq!(
            p.life_state, LIFE_ALIVE,
            "续关原地复活（玩法刀：RESPAWNING 退役）"
        );
        assert_eq!(p.x, Fx::from_int(-40), "位置不动");
        assert_eq!(p.deaths, 3, "偏差值不因续关洗白");
        assert_eq!(p.continues, 1);
        assert_eq!(p.lives, ld.lives);
        assert_eq!(p.bombs, ld.bombs);
        assert_eq!(p.score, 1, "分数 = 续关次数");
        assert_eq!(p.power, 250, "power 不动");
        assert_eq!(p.invuln, RESPAWN_INVULN);
        // 按住不放不连环：再走一帧仍是 1 次
        step_t(&mut w, &keys(BTN_CONTINUE));
        assert_eq!(w.body.players[0].continues, 1);
        // 饱和
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = LIFE_GAMEOVER;
        w.body.players[0].continues = u8::MAX;
        step_t(&mut w, &keys(BTN_CONTINUE));
        assert_eq!(w.body.players[0].continues, u8::MAX);
    }

    /// 落地写 API（玩法刀 spec §4.3）：偏差值 +1、残机 −1 且下限 1、无敌取 max、active 卡失格；
    /// 越界自机号 → no-op + 违约计数（P4-b）。
    #[test]
    fn rewind_landed_pays_the_death_and_floors_lives_at_one() {
        let mut w = crate::step::World::new(1);
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        w.body.players[0].lives = 3;
        w.body.rewind_landed(0);
        let p = w.body.players[0];
        assert_eq!((p.lives, p.deaths, p.invuln), (2, 1, REWIND_INVULN));
        assert_eq!(w.body.spells[0].capture_ok, 0, "快照带回的资格被作废");
        w.body.players[0].lives = 1;
        w.body.rewind_landed(0);
        assert_eq!(
            w.body.players[0].lives, 1,
            "下限 1：致死与否只在死的那一刻判"
        );
        assert_eq!(w.body.players[0].deaths, 2);
        let cv0 = w.body.diag.contract_viol;
        w.body.rewind_landed(crate::MAX_PLAYERS);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    }

    /// 决死窗口耗尽（玩法刀 spec §4.2）：原地回 ALIVE、30 帧无敌、残机 −1、偏差值 +1，
    /// 同帧发 `EVT_PLAYER_DIED` 与 `EVT_REWIND_REQUESTED{data[0]=hit_frame}`；N−1 帧仍在窗口。
    #[test]
    fn deathwindow_expiry_continues_in_place_and_requests_rewind() {
        use crate::events::{EVT_PLAYER_DIED, EVT_REWIND_REQUESTED};
        use crate::math::Fx;
        use crate::player::{DEATHBOMB_WINDOW, LIFE_ALIVE, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = DEATHBOMB_WINDOW;
        w.body.players[0].hit_frame = 7;
        w.body.players[0].x = Fx::from_int(50);
        w.body.players[0].y = Fx::from_int(300);
        let lives0 = w.body.players[0].lives;
        for _ in 0..(DEATHBOMB_WINDOW - 1) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(
            w.body.players[0].life_state, LIFE_DEATHWINDOW,
            "N−1 帧仍在窗口"
        );
        step_t(&mut w, &InputFrame::empty(0));
        let p = w.body.players[0];
        assert_eq!(p.life_state, LIFE_ALIVE);
        assert_eq!(
            (p.x, p.y),
            (Fx::from_int(50), Fx::from_int(300)),
            "原地，不回场底"
        );
        assert_eq!(p.lives, lives0 - 1);
        assert_eq!(p.deaths, 1);
        assert_eq!(p.invuln, REWIND_INVULN);
        let evs = w.frame_events();
        assert!(evs.iter().any(|e| e.kind == EVT_PLAYER_DIED));
        let req: Vec<_> = evs
            .iter()
            .filter(|e| e.kind == EVT_REWIND_REQUESTED)
            .collect();
        assert_eq!(req.len(), 1);
        assert_eq!((req[0].a_index, req[0].data[0]), (0, 7), "载荷 = hit_frame");
    }

    #[test]
    fn deaths_enters_the_checksum() {
        let mut w = crate::step::World::new(1);
        let c0 = w.checksum();
        w.body.players[0].deaths = 1;
        assert_ne!(w.checksum(), c0);
    }

    /// 命尽 → GAMEOVER（而非重生），且 GAMEOVER 后自机冻结（不移动、不发弹）。
    ///
    /// 金向量压不到这条路径 —— M0-9 复审实测：它的自机只死 2 次、`lives` 最低停在 1，
    /// `commit_death` 的 GAMEOVER 分支一次都没跑过。上面那个测试走的是 3→2 的原地继续臂。
    /// 这里把 `lives` 设成 1，逼出 `lives==0` 那一支，并连同它的两个守卫一起钉住：
    /// `match` 的 `LIFE_GAMEOVER => continue` 臂，以及 `commit_death` 之后的 GAMEOVER 复查。
    #[test]
    fn last_life_death_enters_gameover_and_freezes_player() {
        use crate::input::{BTN_RIGHT, BTN_SHOT, InputFrame};
        use crate::player::{DEATHBOMB_WINDOW, LIFE_DEATHWINDOW, LIFE_GAMEOVER};
        let mut w = crate::step::World::new(1);
        w.body.players[0].lives = 1; // 最后一条命
        w.body.players[0].life_state = LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = DEATHBOMB_WINDOW;
        let x0 = w.body.players[0].x;

        // 窗口耗尽 → commit_death → lives 0 → GAMEOVER（不遡行）
        let mut requested = false;
        for f in 0..DEATHBOMB_WINDOW as u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            requested |= w
                .frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_REWIND_REQUESTED);
        }
        assert_eq!(w.body.players[0].life_state, LIFE_GAMEOVER);
        assert_eq!(w.body.players[0].lives, 0);
        assert!(!requested, "残机耗尽不遡行");
        assert_eq!(w.body.players[0].deaths, 1, "最后一条命也计偏差值");

        // GAMEOVER 后：给足输入也不该动、不该发弹
        let mut f = InputFrame::empty(100);
        f.actions[0].buttons = BTN_RIGHT | BTN_SHOT;
        for _ in 0..10 {
            crate::world::test_support::step_t(&mut w, &f);
        }
        assert_eq!(w.body.players[0].x, x0, "GAMEOVER 后不该移动");
        assert_eq!(w.body.shots.iter_alive().count(), 0, "GAMEOVER 后不该发弹");
    }

    /// 相位 3 解释器逐档弹数（M0-17 T4 判别腿①③）：手动押相位（跳过 integrate，读原生出生点，
    /// 免"弹已飞一帧"的位移噪声——出生点断言要逐位精确）。tier0 一路直射；tier2（power=250）
    /// 两路 `dx=∓8px`；tier4（power=400）三路本体 + 1 路子机，子机出生点 =
    /// 自机位 + `option_pos[4][0]`(-20px, 8px) 逐位。
    ///
    /// 变异腿：若解释器 tier 索引钉死恒 0，tier2/tier4 两处弹数会退化成 1——本测试判此。
    #[test]
    fn shottype_tier_bullet_counts() {
        use crate::input::BTN_SHOT;
        use crate::math::Fx;

        // tier0（power=0，默认出生值）：一路直射，首帧持住立即出 1 弹。
        {
            let mut w = crate::step::World::new(1);
            w.body.players[0].input = BTN_SHOT;
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = crate::world::PH_PLAYERS;
            }
            w.body.update_players(&crate::tables::TABLES_V0);
            assert_eq!(w.body.shots.iter_alive().count(), 1, "tier0 一路直射");
        }

        // tier2（power=250）：两路，dx = ∓8px 逐位。
        {
            let mut w = crate::step::World::new(1);
            w.body.players[0].power = 250;
            w.body.players[0].input = BTN_SHOT;
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = crate::world::PH_PLAYERS;
            }
            w.body.update_players(&crate::tables::TABLES_V0);
            assert_eq!(w.body.shots.iter_alive().count(), 2, "tier2 两路");
            let px = w.body.players[0].x;
            let mut xs: Vec<i32> = w
                .body
                .shots
                .iter_alive()
                .map(|i| w.body.shots.x[i].raw())
                .collect();
            xs.sort();
            let mut want = vec![(px - Fx::from_int(8)).raw(), (px + Fx::from_int(8)).raw()];
            want.sort();
            assert_eq!(xs, want, "tier2 两路 x 偏移 ∓8px 逐位");
        }

        // tier4（power=400）：3 路本体 + 1 路子机 = 4 弹；子机出生点 = 自机位 + (-20px, 8px)。
        {
            let mut w = crate::step::World::new(1);
            w.body.players[0].power = 400;
            w.body.players[0].input = BTN_SHOT;
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = crate::world::PH_PLAYERS;
            }
            w.body.update_players(&crate::tables::TABLES_V0);
            assert_eq!(
                w.body.shots.iter_alive().count(),
                4,
                "tier4 三路本体 + 1 路子机"
            );
            let px = w.body.players[0].x;
            let py = w.body.players[0].y;
            let want_x = (px + Fx::from_int(-20)).raw();
            let want_y = (py + Fx::from_int(8)).raw();
            let hit =
                w.body.shots.iter_alive().any(|i| {
                    w.body.shots.x[i].raw() == want_x && w.body.shots.y[i].raw() == want_y
                });
            assert!(hit, "子机弹出生点必须命中自机位 + option_pos 偏移逐位");
        }
    }

    /// `shot_timer` 计时相位 + 松手清零（M0-17 T4 判别腿②）：tier0 `interval=4/delay=0`。
    /// 持续持住命中相位 `k % 4 == 0`（首帧 k=0 立即出、第 4 帧再出）；松 1 帧清零后再持，
    /// 首发延迟须与初次一致（若松手不清零，这里会因残留相位错开而不在本帧命中，判此变异）。
    #[test]
    fn shot_timer_phase_and_release_reset() {
        use crate::input::{BTN_SHOT, InputFrame};

        let hold = |f: u32| {
            let mut inp = InputFrame::empty(f);
            inp.actions[0].buttons = BTN_SHOT;
            inp
        };

        let mut w = crate::step::World::new(1);
        crate::world::test_support::step_t(&mut w, &hold(0));
        assert_eq!(
            w.body.shots.iter_alive().count(),
            1,
            "首帧持住立即出弹（先判后加，delay=0）"
        );
        for f in 1..4u32 {
            crate::world::test_support::step_t(&mut w, &hold(f));
        }
        assert_eq!(
            w.body.shots.iter_alive().count(),
            1,
            "interval 未到（k=1..3）不追加"
        );
        crate::world::test_support::step_t(&mut w, &hold(4));
        assert_eq!(
            w.body.shots.iter_alive().count(),
            2,
            "第 4 帧（k=4，命中 interval）再出一发"
        );

        // 松 1 帧 → shot_timer 清零；再持 → 首发延迟同初次（本帧立即出，不必再等 interval）。
        let count_before = w.body.shots.iter_alive().count();
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(5));
        crate::world::test_support::step_t(&mut w, &hold(6));
        assert_eq!(
            w.body.shots.iter_alive().count(),
            count_before + 1,
            "松手清零后再持——首发延迟同初次"
        );
    }

    /// focus 索引真判别（M0-17 T4，终审前复审 Important 补强）：v0 正式表两焦点槽共享指针，
    /// 拿它测 focus 索引是装饰断言（focus 恒 0 的 bug 照样绿）。本测试造**定制表**——tier 0
    /// 无焦 1 路 / 聚焦 2 路——持 `BTN_SLOW` 出 2 弹、不持出 1 弹，把"解释器真的用 focus=1
    /// 索引"变成可红命题。
    #[test]
    fn focus_indexes_focused_set() {
        use crate::input::{BTN_SHOT, BTN_SLOW};
        use crate::tables::{Shooter, TABLES_V0, WorldTables};

        const S: Shooter = Shooter {
            interval: 4,
            delay: 0,
            dx: crate::math::Fx::ZERO,
            dy: crate::math::Fx::ZERO,
            angle: crate::math::Angle(49152),
            speed: crate::math::Fx::from_int(12),
            damage: 1,
            radius: crate::math::Fx::from_int(4),
            sprite: 0,
            option: 0,
            flags: 0,
        };
        static UNFOCUSED_1WAY: [Shooter; 1] = [S];
        static FOCUSED_2WAY: [Shooter; 2] = [S, S];
        let mut t = WorldTables {
            content_hash: 0,
            characters: TABLES_V0.characters.clone(),
            item_cfg: TABLES_V0.item_cfg,
            drop_tables: TABLES_V0.drop_tables.clone(),
            item_gravity: TABLES_V0.item_gravity,
            color_stride: TABLES_V0.color_stride,
            appearances: TABLES_V0.appearances.clone(),
        };
        t.characters[0].shot.sets[0] = [Box::new(UNFOCUSED_1WAY), Box::new(FOCUSED_2WAY)];

        let run = |slow: bool| -> usize {
            let mut w = crate::step::World::new(1);
            w.body.players[0].input = if slow { BTN_SHOT | BTN_SLOW } else { BTN_SHOT };
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = crate::world::PH_PLAYERS;
            }
            w.body.update_players(&t);
            w.body.shots.iter_alive().count()
        };
        assert_eq!(run(false), 1, "focus=0 读无焦列表（1 路）");
        assert_eq!(run(true), 2, "focus=1 读聚焦列表（2 路）——索引判别腿");
    }

    /// P4-b：power 直写越 POWER_MAX（导演/ECL 的合法通道）不越 sets 表界——钳到满档
    /// 照常发弹不 panic（M0-17 终审实证 power=500 未钳时 release 下 index OOB）。
    #[test]
    fn overpower_clamps_to_top_tier_no_panic() {
        use crate::input::BTN_SHOT;
        let mut w = crate::step::World::new(1);
        w.body.players[0].power = 999;
        w.body.players[0].input = BTN_SHOT;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_PLAYERS;
        }
        w.body.update_players(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.shots.iter_alive().count(),
            4,
            "钳到 tier 4：三路 + 子机照常齐射"
        );
    }

    // ── 停止自机入口（玩法刀 2026-09-14：时停 + bomb 合一，X = BTN_BOMB）──────────

    fn press(w: &mut crate::step::World, buttons: u32) {
        let mut input = crate::input::InputFrame::empty(w.frame());
        input.actions[0].buttons = buttons;
        crate::step::step(
            w,
            &crate::tables::TABLES_V0,
            &crate::ecl::image::EclImage::empty(),
            &input,
        );
    }

    /// 门禁 + 效果：扣一发库存、写玩家技能倒计时、不碰 ECL 演出格。两件都断——
    /// 「扣费但没生效」「生效但没扣费」各能溜过只断其一的写法。
    #[test]
    fn stop_triggers_charges_one_and_freezes() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "应扣一发");
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
        assert_eq!(w.body.freeze_left[1], 0, "不得碰 ECL 演出那一格");
    }

    /// 停止期间再按（松手重按 = 真沿）= no-op 且不扣、不刷新倒计时。
    #[test]
    fn stop_reentry_is_free_noop() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        press(&mut w, 0);
        let left = w.body.freeze_left[0];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "停止中再按不得扣");
        assert!(w.body.freeze_left[0] < left, "也不得刷新倒计时");
    }

    #[test]
    fn stop_without_stock_does_nothing() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 0;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.freeze_left[0], 0);
    }

    /// A 组被 ECL 演出定住时发不出——A 组门禁自动给的。
    #[test]
    fn stop_is_unavailable_while_the_actor_is_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        w.body.freeze_left = [0, 10];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.freeze_left[0], 0, "被定住期间不得发动");
        assert_eq!(w.body.players[0].bombs, 2, "也不得扣");
    }

    /// 真实按键 ⇒ 世界恰好少走 `TIMESTOP_FRAMES` 帧（N±1 判别，观测面 = 有速度的敌弹）。
    /// 弹放在远离自机处（x=150），免得被触碰消弹吃掉。
    #[test]
    fn real_button_press_skips_exactly_timestop_frames() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 150, 100);
        let bi = w.body.bullets.get(h).unwrap();
        w.body.bullets.vy[bi] = crate::math::Fx::from_int(1);
        let y0 = w.body.bullets.y[bi];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.bullets.y[bi], y0, "触发当帧起即已冻");
        for k in 1..crate::player::TIMESTOP_FRAMES {
            press(&mut w, 0);
            assert_eq!(w.body.bullets.y[bi], y0, "第 {k} 帧仍应冻结");
        }
        press(&mut w, 0);
        assert_ne!(
            w.body.bullets.y[bi], y0,
            "第 TIMESTOP_FRAMES+1 帧必须已解除"
        );
        assert_eq!(w.body.freeze_left[0], 0);
    }

    /// 按住跨过整个冻结窗口（含解冻那帧）只扣一发——查电平的实现会在解冻帧再点一次。
    #[test]
    fn holding_through_expiry_consumes_exactly_one_charge() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        for _ in 0..=crate::player::TIMESTOP_FRAMES {
            press(&mut w, BTN_BOMB);
        }
        assert_eq!(w.body.players[0].bombs, 1, "全程按住只扣一发");
        assert_eq!(w.body.freeze_left[0], 0, "不得被电平误判重新点燃");
    }

    /// 与上条互补：松手、等窗口跑完再按 = 合法第二发。
    #[test]
    fn genuine_second_press_after_release_fires_again() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        for _ in 0..crate::player::TIMESTOP_FRAMES {
            press(&mut w, 0);
        }
        assert_eq!(w.body.freeze_left[0], 0);
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "合法第二发照常发动");
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
    }

    /// deathstop：决死窗口内按 X → 拨回 ALIVE、清窗口计时、扣一发、**不扣命**。
    #[test]
    fn deathstop_inside_the_window_revives_without_costing_a_life() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
        assert_eq!(w.body.players[0].lives, lives0, "不扣命");
        assert_eq!(w.body.players[0].state_timer, 0);
        assert_eq!(w.body.players[0].bombs, 0);
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
    }

    /// 窗口耗尽（命已扣）之后再按停止：命不会退回。与上一条成对才有判别力。
    #[test]
    fn stop_after_the_window_closed_cannot_undo_the_death() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0); // 窗口耗尽 → commit_death
        assert_eq!(w.body.players[0].lives, lives0 - 1, "已经扣命");
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "命不会退");
    }

    /// 停止 ⇒ active 符卡当场失格（资格轮询住 settle，冻结期间不跑，必须在触发点写）。
    #[test]
    fn stop_voids_the_spell_capture_at_trigger() {
        let mut w = crate::step::World::new(1);
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        // (slot, boss, spell_id, time_limit, bonus0, flags, hp_threshold)
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        assert_ne!(w.body.spells[0].capture_ok, 0, "前提：开卡时资格在");
        w.body.players[0].bombs = 1;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.spells[0].capture_ok, 0, "停止即失格");
    }

    /// 真按键端到端：身上压一颗弹，按 X → 同帧冻结 + 触碰消掉 + 仍 ALIVE。
    #[test]
    fn stop_by_button_clears_the_bullet_under_the_player() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        bullet_at(&mut w, 0, 384);
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
    }

    /// 落点快照停在决死窗口里（关底封印 / 窗口内读档）：落地拨回 ALIVE、清窗口计时——
    /// 否则窗口在恢复出的世界里再耗尽一次，连环遡行直到 GAMEOVER（复审 Important 1）。
    #[test]
    fn rewind_landed_on_a_deathwindow_snapshot_revives() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 3;
        w.body.rewind_landed(0);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
        assert_eq!(w.body.players[0].state_timer, 0);
    }

    /// 死亡帧 A 组不跑：同帧按 X 不得在扣命之后再扣一发停止（复审 Minor 3）。
    #[test]
    fn death_frame_skips_actions() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        let lives0 = w.body.players[0].lives;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "窗口耗尽已扣命");
        assert_eq!(w.body.players[0].bombs, 1, "死亡帧不发动停止");
        assert_eq!(w.body.freeze_left[0], 0);
    }

    /// JUMPING / GAMEOVER 下按 X 无效。
    #[test]
    fn stop_is_noop_while_jumping_or_gameover() {
        for st in [LIFE_JUMPING, crate::player::LIFE_GAMEOVER] {
            let mut w = crate::step::World::new(1);
            w.body.players[0].bombs = 1;
            w.body.players[0].life_state = st;
            w.body.players[0].state_timer = JUMP_FRAMES;
            press(&mut w, BTN_BOMB);
            assert_eq!(w.body.players[0].bombs, 1, "state {st}");
            assert_eq!(w.body.freeze_left[0], 0, "state {st}");
        }
    }

    /// 失格只动 active 槽。
    #[test]
    fn void_spell_captures_leaves_inactive_slots_alone() {
        let mut w = crate::step::World::new(1);
        w.body.spells[1].capture_ok = 1; // active == 0 的槽
        w.body.void_spell_captures();
        assert_eq!(w.body.spells[1].capture_ok, 1);
    }

    // ── 经典机体（经典机体刀 2026-09-15）：机体 1 = Kit::Classic ────────────────

    /// 机体 1 世界：自机换成 `spawn(1, …)`，其余同 `World::new`。
    fn classic_world() -> Box<crate::step::World> {
        let mut w = crate::step::World::new(1);
        w.body.players[0] =
            crate::player::PlayerState::spawn(1, &crate::tables::TABLES_V0.characters[1]);
        w
    }

    fn classic_bomb() -> &'static crate::tables::BombCfg {
        match &crate::tables::TABLES_V0.characters[1].kit {
            crate::tables::Kit::Classic(b) => b,
            crate::tables::Kit::Chronos => unreachable!("机体 1 必须是 Classic"),
        }
    }

    /// 机体 1 复用机体 0 的火力（spec §2.4）：按住 SHOT 一帧出弹数与机体 0 相同且 > 0。
    #[test]
    fn classic_character_fires_the_same_shots_as_character_0() {
        let count = |mut w: Box<crate::step::World>| {
            press(&mut w, crate::input::BTN_SHOT);
            w.body.shots.iter_alive().count()
        };
        let (c0, c1) = (count(crate::step::World::new(1)), count(classic_world()));
        assert!(c0 > 0, "前提：机体 0 按下当帧出弹");
        assert_eq!(c1, c0, "机体 1 必须与机体 0 同样出弹");
    }

    /// ①② 成对：窗口内能救且不扣命；窗口耗尽后按 X 救不回（自机此时已重生，合法起爆照扣 bomb）。
    #[test]
    fn classic_deathbomb_inside_the_window_revives_without_costing_a_life() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
        assert_eq!(w.body.players[0].lives, lives0, "决死救人不扣命");
        assert_eq!(w.body.players[0].state_timer, 0);
        assert_eq!(w.body.players[0].bombs, 0);
        assert_eq!(w.body.players[0].bomb_timer, classic_bomb().frames);
    }

    #[test]
    fn classic_bomb_after_the_window_closed_cannot_undo_the_death() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0); // 窗口耗尽 → commit_death
        assert_eq!(w.body.players[0].lives, lives0 - 1, "已经扣命");
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "命不会退");
        assert_eq!(w.body.players[0].deaths, 1);
        assert_eq!(w.body.players[0].bombs, 0, "重生后合法起爆照扣 bomb");
    }

    /// 伤害圆几何判别：圈内敌掉血、圈外不掉；圆心 = 起爆点 (0,200) ≠ 场心 (0,224)。
    #[test]
    fn classic_bomb_damage_field_is_at_the_cast_point_and_hits_only_inside() {
        use crate::math::Fx;
        let mut w = classic_world();
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.players[0].bombs = 1;
        let near = crate::world::test_support::spawn_enemy(&mut w, 0, 240, 1000); // 距 40
        let far = crate::world::test_support::spawn_enemy(&mut w, 0, 40, 1000); // 距 160 > 120+16
        let (ni, fi) = (
            w.body.enemies.get(near).unwrap(),
            w.body.enemies.get(far).unwrap(),
        );
        let (nhp, fhp) = (w.body.enemies.hp[ni], w.body.enemies.hp[fi]);
        press(&mut w, BTN_BOMB);
        let f = w
            .body
            .fields
            .iter_alive()
            .find(|&i| w.body.fields.flags[i] & crate::field::FIELD_DAMAGE != 0)
            .expect("应铺了伤害 field");
        assert_eq!(
            (w.body.fields.x[f], w.body.fields.y[f]),
            (Fx::ZERO, Fx::from_int(200))
        );
        press(&mut w, 0);
        assert!(w.body.enemies.hp[ni] < nhp, "圈内敌必须掉血");
        assert_eq!(w.body.enemies.hp[fi], fhp, "圈外敌不得掉血");
    }

    /// 持续消弹：起爆后第 60 帧新来的弹也被消掉（`life = 1` 的错实现会红）。
    #[test]
    fn classic_bomb_clear_field_keeps_clearing_for_its_whole_duration() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        press(&mut w, BTN_BOMB);
        for _ in 0..59 {
            press(&mut w, 0);
        }
        bullet_at(&mut w, 0, 200);
        press(&mut w, 0);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "整段期间新弹也该被消掉"
        );
    }

    /// 沿检测三件：按住不连环 / 结束后真沿再发 / 进行中再按 no-op 不扣不刷新。
    #[test]
    fn classic_holding_the_bomb_key_does_not_chain_bomb() {
        let mut w = classic_world();
        w.body.players[0].bombs = 3;
        let frames = classic_bomb().frames;
        for _ in 0..=(2 * frames + 4) {
            press(&mut w, BTN_BOMB);
        }
        assert_eq!(w.body.players[0].bombs, 2, "全程按住只该起爆一次");
    }

    #[test]
    fn classic_genuine_second_bomb_after_release_fires_again() {
        let mut w = classic_world();
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        for _ in 0..classic_bomb().frames {
            press(&mut w, 0);
        }
        assert_eq!(w.body.players[0].bomb_timer, 0, "本段应已结束");
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "新的真沿必须照常起爆");
    }

    #[test]
    fn classic_pressing_bomb_while_one_is_active_is_a_free_noop() {
        let mut w = classic_world();
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        press(&mut w, 0);
        let left = w.body.players[0].bomb_timer;
        assert!(left > 0);
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "效果中再按不得扣");
        assert!(w.body.players[0].bomb_timer < left, "不得刷新计时");
    }

    /// 计时恰好 `frames` 帧归零：触发帧不被自己减，第 frames−1 帧仍 >0，第 frames 帧 ==0。
    #[test]
    fn classic_bomb_timer_runs_out_exactly_after_frames() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let frames = classic_bomb().frames;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_timer, frames, "触发当帧不被自己减掉");
        for k in 1..frames {
            press(&mut w, 0);
            assert_ne!(w.body.players[0].bomb_timer, 0, "第 {k} 帧仍应在效果中");
        }
        press(&mut w, 0);
        assert_eq!(w.body.players[0].bomb_timer, 0);
    }

    /// 门禁：无库存无效；ECL 演出冻结（A 组）发不出、不扣。
    #[test]
    fn classic_bomb_gates_stock_and_actor_freeze() {
        let mut w = classic_world();
        w.body.players[0].bombs = 0;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_timer, 0);
        assert_eq!(w.body.fields.iter_alive().count(), 0, "不得铺任何作用区");
        let mut w = classic_world();
        w.body.players[0].bombs = 2;
        w.body.freeze_left = [0, 10];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_timer, 0, "被定住期间不得发动");
        assert_eq!(w.body.players[0].bombs, 2);
    }

    /// 空 `fields` 合法：不铺区，但扣库存、进效果段、给无敌（触发帧不被自减）。
    #[test]
    fn classic_bomb_with_no_fields_still_grants_invulnerability() {
        let mut t = crate::tables::build_tables_v0();
        let crate::tables::Kit::Classic(b) = &mut t.characters[1].kit else {
            unreachable!()
        };
        b.fields = Box::new([]);
        let invuln = b.invuln;
        let mut w = crate::step::World::new_with_tables(1, &t);
        w.body.players[0] = crate::player::PlayerState::spawn(1, &t.characters[1]);
        w.body.players[0].bombs = 1;
        let mut input = InputFrame::empty(w.frame());
        input.actions[0].buttons = BTN_BOMB;
        crate::step::step(&mut w, &t, &crate::ecl::image::EclImage::empty(), &input);
        assert_eq!(w.body.players[0].bombs, 0);
        assert_ne!(w.body.players[0].bomb_timer, 0);
        assert_eq!(w.body.fields.iter_alive().count(), 0);
        assert_eq!(w.body.players[0].invuln, invuln);
    }

    /// 起爆当帧全屏吸道具 + 当帧符卡失格。
    #[test]
    fn classic_bomb_attracts_items_and_voids_the_spell_capture() {
        let mut w = classic_world();
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        let h = w.body.drop_item(
            crate::math::Fx::ZERO,
            crate::math::Fx::from_int(60),
            crate::items::ITEM_POWER,
            &crate::tables::TABLES_V0,
        );
        let ii = w.body.items.get(h).unwrap();
        w.body.players[0].bombs = 1;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.items.magnet_to[ii], 0, "起爆当帧应全场上锁到自机 0");
        assert_eq!(w.body.spells[0].capture_ok, 0, "起爆后本卡不予收卡");
    }

    /// 跨套件判别（X 键）：同一按键在机体 0 = 停止、机体 1 = bomb。防分派接反/写死一边。
    #[test]
    fn x_key_dispatches_by_kit() {
        let mut chronos = crate::step::World::new(1);
        chronos.body.players[0].bombs = 1;
        press(&mut chronos, BTN_BOMB);
        assert_eq!(chronos.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
        assert_eq!(chronos.body.players[0].bomb_timer, 0);
        assert_eq!(chronos.body.fields.iter_alive().count(), 0);

        let mut classic = classic_world();
        classic.body.players[0].bombs = 1;
        press(&mut classic, BTN_BOMB);
        assert_eq!(classic.body.freeze_left[0], 0, "Classic 不得停止");
        assert_eq!(classic.body.players[0].bomb_timer, classic_bomb().frames);
        assert_eq!(classic.body.fields.iter_alive().count(), 2);
    }

    /// `bomb_timer` 进校验和（P6）。
    #[test]
    fn bomb_timer_enters_the_checksum() {
        let mut w = classic_world();
        let c0 = w.checksum();
        w.body.players[0].bomb_timer = 1;
        assert_ne!(w.checksum(), c0);
    }

    /// 容量闸：rank-3 峰值 814 弹下起 bomb 跑满整段，道具池不溢出。若变红**不要现场调池 cap**，记录实测留给人裁定。
    #[test]
    fn classic_bomb_at_rank3_peak_bullet_count_does_not_overflow_item_pool() {
        use crate::world::POOL_ITEM;
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        for _ in 0..814 {
            bullet_at(&mut w, 0, 224);
        }
        press(&mut w, BTN_BOMB);
        for _ in 1..classic_bomb().frames {
            press(&mut w, 0);
        }
        assert_eq!(w.body.diag.pool_full[POOL_ITEM], 0);
    }

    /// Classic 死亡：窗口耗尽 → 场底 (0,384)、ALIVE、RESPAWN_INVULN、残机 −1、偏差 +1、**无遡行请求**。
    #[test]
    fn classic_death_respawns_at_field_bottom_without_rewind_request() {
        use crate::math::Fx;
        let mut w = classic_world();
        w.body.players[0].x = Fx::from_int(-100);
        w.body.players[0].y = Fx::from_int(150);
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0);
        let p = w.body.players[0];
        assert_eq!((p.x, p.y), (Fx::ZERO, Fx::from_int(384)), "场底中心");
        assert_eq!(p.life_state, crate::player::LIFE_ALIVE);
        assert_eq!(p.invuln, crate::player::RESPAWN_INVULN);
        assert_eq!((p.lives, p.deaths), (lives0 - 1, 1));
        assert!(
            w.frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_PLAYER_DIED),
            "照发 PlayerDied"
        );
        assert!(
            !w.frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_REWIND_REQUESTED),
            "Classic 不得发遡行请求"
        );
    }

    #[test]
    fn classic_last_life_death_enters_gameover() {
        let mut w = classic_world();
        w.body.players[0].lives = 1;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_GAMEOVER);
    }

    /// 跨套件判别（死亡）：机体 0 原地 + REWIND_INVULN + 请求；机体 1 场底 + RESPAWN_INVULN + 无请求。
    #[test]
    fn death_dispatches_by_kit() {
        use crate::math::Fx;
        let die = |mut w: Box<crate::step::World>| {
            w.body.players[0].x = Fx::from_int(-100);
            w.body.players[0].y = Fx::from_int(150);
            w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
            w.body.players[0].state_timer = 1;
            press(&mut w, 0);
            let asked = w
                .frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_REWIND_REQUESTED);
            (
                w.body.players[0].x,
                w.body.players[0].y,
                w.body.players[0].invuln,
                asked,
            )
        };
        assert_eq!(
            die(crate::step::World::new(1)),
            (Fx::from_int(-100), Fx::from_int(150), REWIND_INVULN, true)
        );
        assert_eq!(
            die(classic_world()),
            (
                Fx::ZERO,
                Fx::from_int(384),
                crate::player::RESPAWN_INVULN,
                false
            )
        );
    }

    /// Classic 按 C：永不进 JUMPING、`jump_cd` 不动；同帧 X 照常 bomb（C 不吞 X）。
    #[test]
    fn classic_c_key_is_a_noop() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        for _ in 0..(JUMP_FRAMES + 2) {
            press(&mut w, BTN_JUMP);
            assert_ne!(w.body.players[0].life_state, LIFE_JUMPING);
        }
        assert_eq!(w.body.players[0].jump_cd, 0);
        press(&mut w, 0);
        press(&mut w, BTN_JUMP | BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "C+X 同帧 X 照常起爆");
    }

    /// 窗口耗尽那一帧按 X：死亡已结算（C 组先于 A 组，死亡帧 A 组跳过），不得扣 bomb、不得起爆。
    #[test]
    fn classic_bomb_on_the_expiry_frame_is_too_late_and_free() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "耗尽帧已扣命");
        assert_eq!(w.body.players[0].deaths, 1);
        assert_eq!(w.body.players[0].bombs, 1, "死亡帧 A 组不跑，不扣 bomb");
        assert_eq!(w.body.players[0].bomb_timer, 0, "也不起爆");
    }
}
