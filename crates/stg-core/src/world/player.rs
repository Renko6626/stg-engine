//! 相位 1 · 输入译码 + 相位 3 · 自机更新（D6/A8）。
//!
//! **命名**：`crate::player` 是 `PlayerState` 数据模块（`bullets`/`enemy`/`field`/`shots` 的同辈）；
//! 本模块是自机的**相位逻辑**。二者并存，路径区分。
//!
//! **生死状态机的触发与计时分家**：本相位（3）独占**全部计时**（决死窗口倒数 → `commit_death`
//! → 重生 → 无敌耗尽 → Alive）；**中弹触发**（Alive → DeathWindow）在 settle（相位 7）。
//! 因相位 3 早于 7，中弹在帧尾定、窗口从次帧起数 —— 这 1 帧错位正是决死窗口的语义。

use super::WorldBody;
use crate::events::Event;
use crate::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SLOW, BTN_UP};
use crate::math::Fx;
use crate::player::{
    LIFE_ABSENT, LIFE_ALIVE, LIFE_DEATHWINDOW, LIFE_GAMEOVER, LIFE_RESPAWNING, RESPAWN_INVULN,
};
use crate::shots::ShotInit;
use crate::tables::WorldTables;

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
            if matches!(self.players[i].life_state, LIFE_ABSENT | LIFE_GAMEOVER) {
                continue;
            }
            // ── C 组：生死状态机计时（A4 相位 3 职责）
            if !scene {
                match self.players[i].life_state {
                    LIFE_DEATHWINDOW => {
                        // bomb 救人 stub：本切片无 bomb 输入 → 窗口必耗尽。
                        if self.players[i].state_timer > 0 {
                            self.players[i].state_timer -= 1;
                        }
                        if self.players[i].state_timer == 0 {
                            self.commit_death(i);
                        }
                    }
                    LIFE_RESPAWNING => {
                        if self.players[i].invuln > 0 {
                            self.players[i].invuln -= 1;
                        }
                        if self.players[i].invuln == 0 {
                            self.players[i].life_state = LIFE_ALIVE;
                        }
                    }
                    LIFE_ALIVE => {
                        if self.players[i].invuln > 0 {
                            self.players[i].invuln -= 1; // bomb 无敌（本切片恒 0）
                        }
                    }
                    _ => {}
                }
                // bomb 计时归 C 组（与 invuln、决死窗口同属"世界对自机的裁决"）——时停期间
                // bomb 不流逝、不浪费无敌帧（spec §10.2）。与状态机无关，任何 life_state 下
                // 只要 bomb_timer 非零就照数，触发帧本身在 A 组之后才写入 timer，故不会被
                // 这里自己减掉（触发帧 C 组先跑到这时 timer 仍是旧值 0）。
                if self.players[i].bomb_timer > 0 {
                    self.players[i].bomb_timer -= 1;
                    if self.players[i].bomb_timer == 0 {
                        self.players[i].bomb_phase = 0;
                    }
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
            self.try_time_stop(i);
            self.try_bomb(i, tables);
            self.move_player(i, tables);
            // 角色模块静态分发点（A8"shottype 类似物"）：现仅 character 0，将来各角色一臂。
            #[allow(clippy::single_match)]
            match self.players[i].character_id {
                0 => self.char0_update_shot(i, tables),
                _ => {}
            }
        }
    }

    /// 决死窗口耗尽的死亡连带结算（世界侧固定，D6）：lives−1、PlayerDied、重生或 game over。
    fn commit_death(&mut self, i: usize) {
        self.players[i].lives = self.players[i].lives.saturating_sub(1);
        let ev = Event {
            kind: crate::events::EVT_PLAYER_DIED,
            a_index: i as u16,
            a_gen: 0,
            x: self.players[i].x,
            y: self.players[i].y,
            data: [self.players[i].lives as i32, 0],
        };
        self.push_event(ev);
        // 掉 power / power 道具回撒 → 道具池切片（此处暂不动 power）。
        if self.players[i].lives == 0 {
            self.players[i].life_state = LIFE_GAMEOVER;
        } else {
            self.players[i].life_state = LIFE_RESPAWNING;
            self.players[i].x = Fx::ZERO; // 场底中心（与 spawn 一致）
            self.players[i].y = Fx::from_int(384);
            self.players[i].invuln = RESPAWN_INVULN;
            self.players[i].state_timer = 0;
        }
    }

    /// 时停触发（A 组）。门禁四条：动作位**上升沿**（`pressed_edge`）+ 资源 > 0 +
    /// 该能力未在进行 + 自机 ALIVE。
    ///
    /// **两条门禁各管一段、缺一不可**（复审 2026-09-04 纠偏——此前误以为
    /// `freeze_left[0] != 0` 能替代真正的沿检测，被"按住跨过整个冻结窗口"戳穿）：
    ///
    /// - `pressed_edge` 管"按下瞬间"：`decode_input` 对任何位都不做沿译码
    ///   （`players[i].input` 就是当帧原始电平，按住则连续多帧为 1），真正的"这一帧是不是
    ///   刚按下"必须靠比较 `input`/`prev_input` 求出，见 `pressed_edge` 文档。
    /// - `freeze_left[0] != 0` 管"时停中再按"：这是裁定 #6 的语义（no-op 且不扣资源），
    ///   与"防抖"是两件不同的事——即便沿检测完全正确，玩家也可能在时停生效期间又按了
    ///   一次新的沿（松开重按），这条门禁负责把那次新沿也挡掉。
    ///
    /// **只用 `freeze_left[0]!=0` 当防抖为什么不够**：`freeze_left[0]` 只在
    /// `TIMESTOP_FRAMES` 窗口内非零。若玩家从触发帧起持续按住不放、跨过整个窗口，
    /// 第 `TIMESTOP_FRAMES` 帧 `freeze_left[0]` 归零而 `input` 仍是同一次物理按压的延续
    /// （电平仍为 1，`decode_input` 不知道"这是不是新按下的"）——只查电平的旧实现会在
    /// 那一帧误判成"新的一次触发"，按住不放即可连环耗尽全部资源。判别测试见
    /// `holding_through_expiry_consumes_exactly_one_charge`。
    ///
    /// 时停期间再按 = **no-op 且不扣资源**（裁定 #6）。
    fn try_time_stop(&mut self, i: usize) {
        if !self.pressed_edge(i, crate::input::BTN_TIMESTOP)
            || self.players[i].time_stops == 0
            || self.freeze_left[0] != 0
            || self.players[i].life_state != LIFE_ALIVE
        {
            return;
        }
        self.players[i].time_stops -= 1;
        self.freeze_left[0] = crate::player::TIMESTOP_FRAMES;
    }

    /// bomb 触发（A 组）。门禁四条与时停同构，两处不同：
    ///
    /// - **必须用 `pressed_edge`（上升沿），不能查电平**（brief 给的参考实现 `input & BTN_BOMB
    ///   == 0` 是错的，已被本刀否掉）：默认装备 3 颗 bomb，本函数每帧都跑，而 `bomb_phase`
    ///   在计时归零那一帧被 C 组当场清 0——若门禁查电平，第 `frames` 帧 C 组刚清完
    ///   `bomb_phase`、A 组紧接着又看见电平 1，会立刻判定"可以再点一发"，按住不放即可把
    ///   全部存量一帧接一帧烧光。判别测试见 `holding_the_bomb_key_does_not_chain_bomb`。
    /// - **门禁允许 `LIFE_DEATHWINDOW`**（时停只认 `LIFE_ALIVE`）：主动 bomb 与 deathbomb
    ///   救人是同一条路径，只是入口状态不同。
    ///
    /// **deathbomb 为什么不需要"退款"**：进入决死窗口时只改了 `life_state`/`state_timer`，
    /// `lives` 一分未动；真正扣命只发生在 `commit_death`（决死窗口计时耗尽才跑，见本文件
    /// C 组 `LIFE_DEATHWINDOW` 分支）。所以救人 = 把状态拨回 `LIFE_ALIVE` + 清窗口计时，
    /// 天生没有"已经扣了命、现在要还回去"这一步，无退款逻辑可写、也没有可写错的退款逻辑。
    fn try_bomb(&mut self, i: usize, tables: &WorldTables) {
        if !self.pressed_edge(i, crate::input::BTN_BOMB)
            || self.players[i].bombs == 0
            || self.players[i].bomb_phase != 0
            || !matches!(self.players[i].life_state, LIFE_ALIVE | LIFE_DEATHWINDOW)
        {
            return;
        }
        let cfg = &tables.characters[self.players[i].character_id as usize].bomb;
        self.players[i].bombs -= 1;
        self.players[i].bomb_phase = 1;
        self.players[i].bomb_timer = cfg.frames;
        if self.players[i].life_state == LIFE_DEATHWINDOW {
            self.players[i].life_state = LIFE_ALIVE;
            self.players[i].state_timer = 0;
        }
        self.players[i].invuln = cfg.invuln;
        let (px, py) = (self.players[i].x, self.players[i].y);
        // 按声明序铺 field（I4）；`fields` 合法可空（"只给无敌"的表），零轮次循环本身
        // 就是正确行为，不需要额外特判。按索引取而非 `.iter()`，避免给 `cfg`（借自
        // `tables: &WorldTables`）挂上一个跨越 `self.create_field`（&mut self）调用的
        // 活跃迭代器引用——`tables` 与 `self` 本是两个不同对象，理论上不冲突，但按索引
        // 更直白也更贴合"别为绕借用检查在 stg-core 里加堆分配"的红线（本来就不需要堆）。
        for k in 0..cfg.fields.len() {
            let f = tables.characters[self.players[i].character_id as usize]
                .bomb
                .fields[k];
            // 穷尽 match：将来加 BombOrigin 变体而忘了处理 ⇒ 编译不过（D18 手法）。
            let (x, y) = match f.origin {
                crate::tables::BombOrigin::FieldCenter => {
                    (Fx::ZERO, Fx::from_int(super::FIELD_HEIGHT / 2))
                }
                crate::tables::BombOrigin::PlayerAtCast => (px, py),
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
    #[test]
    fn deathwindow_expires_to_respawn_after_window() {
        use crate::input::InputFrame;
        use crate::player::{LIFE_ALIVE, LIFE_RESPAWNING};
        let mut w = crate::step::World::new(1);
        // 手动置决死窗口（模拟已中弹）
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        let lives0 = w.body.players[0].lives;
        // 跑够窗口帧数 → Dead → Respawning
        for _ in 0..crate::player::DEATHBOMB_WINDOW {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_RESPAWNING);
        assert_eq!(w.body.players[0].lives, lives0 - 1);
        assert!(w.body.players[0].invuln > 0);
        // 再跑够无敌帧 → Alive
        for _ in 0..crate::player::RESPAWN_INVULN {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE);
    }

    /// 命尽 → GAMEOVER（而非重生），且 GAMEOVER 后自机冻结（不移动、不发弹）。
    ///
    /// 金向量压不到这条路径 —— M0-9 复审实测：它的自机只死 2 次、`lives` 最低停在 1，
    /// `commit_death` 的 GAMEOVER 分支一次都没跑过。上面那个测试走的是 3→2 的 RESPAWNING 臂。
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

        // 窗口耗尽 → commit_death → lives 0 → GAMEOVER（不是 RESPAWNING）
        for f in 0..DEATHBOMB_WINDOW as u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_GAMEOVER);
        assert_eq!(w.body.players[0].lives, 0);

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

    // ── 时停自机入口（自机能力刀 Task 4）─────────────────────────────────

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

    /// 门禁四条 + 效果。判别力：逐条断言"扣了资源"与"冻了世界"两件，只断其一的话
    /// "扣费但没生效"或"生效但没扣费"各能溜过一条。
    #[test]
    fn time_stop_triggers_and_charges_one_use() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 2;
        press(&mut w, crate::input::BTN_TIMESTOP);
        assert_eq!(w.body.players[0].time_stops, 1, "应扣一次资源");
        assert_eq!(
            w.body.freeze_left[0],
            crate::player::TIMESTOP_FRAMES,
            "应写玩家技能倒计时（相位 3 写、当帧相位 4 起即冻）"
        );
        assert_eq!(w.body.freeze_left[1], 0, "不得碰 ECL 演出那一格");
    }

    /// 时停期间再按 = no-op **且不扣资源**（裁定 #6）。
    #[test]
    fn time_stop_reentry_is_free_noop() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 2;
        press(&mut w, crate::input::BTN_TIMESTOP);
        let left = w.body.freeze_left[0];
        press(&mut w, crate::input::BTN_TIMESTOP);
        assert_eq!(w.body.players[0].time_stops, 1, "时停中再按不得扣资源");
        assert!(
            w.body.freeze_left[0] < left,
            "也不得刷新倒计时（覆盖是 ECL 侧的语义）"
        );
    }

    /// 资源为 0 时按无效。
    #[test]
    fn time_stop_without_charges_does_nothing() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 0;
        press(&mut w, crate::input::BTN_TIMESTOP);
        assert_eq!(w.body.freeze_left[0], 0);
    }

    /// A 组被冻（ECL 演出进行中）时不能发动时停——"你被定住了当然不能用"，
    /// 这条不是特例，是 A 组门禁自动给的。
    #[test]
    fn time_stop_is_unavailable_while_the_actor_is_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 2;
        w.body.freeze_left = [0, 10];
        press(&mut w, crate::input::BTN_TIMESTOP);
        assert_eq!(w.body.freeze_left[0], 0, "被定住期间不得发动");
        assert_eq!(w.body.players[0].time_stops, 2, "也不得扣资源");
    }

    /// Task 3 复审 carryover (a)：Task 3 那批"恰好少走 N 帧"判别测试（`step.rs` 的
    /// `both_freezes_always_expire_and_last_exactly_n_frames`）写的时候相位 3 触发还不
    /// 存在，只能借 `step_with_director` 在相位 2 直接点火。现在真触发有了，改用**真实
    /// 输入位**按一次 `BTN_TIMESTOP`，钉死同一条时序语义："相位 3 写 N ⇒ 世界恰好少走
    /// N 帧"——用一颗有速度的敌弹（C 组）当观测面：按下当帧起飞行冻住，之后
    /// `TIMESTOP_FRAMES − 1` 帧仍冻，第 `TIMESTOP_FRAMES` 帧解除、弹恢复飞行，一帧不多
    /// 一帧不少（只测"变小了"或只测头一帧的话，"恒冻一帧"或"提前/推迟一帧解除"两种
    /// 错实现都能溜过）。
    #[test]
    fn real_button_press_skips_exactly_timestop_frames() {
        let mut w = crate::step::World::new(1);
        let h = crate::world::test_support::bullet_at(&mut w, 0, 100);
        let bi = w.body.bullets.get(h).unwrap();
        w.body.bullets.vy[bi] = crate::math::Fx::from_int(1);
        let y0 = w.body.bullets.y[bi];

        press(&mut w, crate::input::BTN_TIMESTOP); // 相位 2 之后即触发，本帧相位 4 起即冻
        assert_eq!(w.body.bullets.y[bi], y0, "触发当帧起即已冻");

        for k in 1..crate::player::TIMESTOP_FRAMES {
            let f = w.frame();
            crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(f));
            assert_eq!(w.body.bullets.y[bi], y0, "第 {k} 帧仍应在冻结中");
        }
        let f = w.frame();
        crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(f));
        assert_ne!(
            w.body.bullets.y[bi], y0,
            "第 TIMESTOP_FRAMES+1 帧必须已解除"
        );
        assert_eq!(w.body.freeze_left[0], 0, "倒计时必须归零");
    }

    /// 复审纠偏（2026-09-04）：只查电平会在解冻边界"自己骗自己"——之前的实现拿
    /// `freeze_left[0] != 0` 当防抖，但那只在 `TIMESTOP_FRAMES` 窗口**内**非零。若玩家从
    /// 触发帧起持续按住不放、跨过整个窗口，第 `TIMESTOP_FRAMES` 帧 `freeze_left[0]`
    /// 归零而 `input` 仍是同一次物理按压的延续（电平仍是 1）——只查电平的旧实现会把
    /// 这一帧误判成"新的一次触发"，按住不放就能连环耗尽全部资源。
    ///
    /// 判别力：起始 2 点资源，从触发帧起连续按住直到跨过 `TIMESTOP_FRAMES`（含解冻那
    /// 一帧本身仍不松手），断言资源**只扣一次**、`freeze_left` 没有被重新点燃——这条对
    /// 旧的纯电平实现是红的（会在解冻帧误触发第二次，`time_stops` 会变成 0）。
    #[test]
    fn holding_through_expiry_consumes_exactly_one_charge() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 2;
        // 触发帧 + 之后连续按住到解冻那一帧（含）：0..=TIMESTOP_FRAMES 共
        // TIMESTOP_FRAMES+1 次按压，覆盖"窗口内每一帧"以及"窗口恰好耗尽的那一帧"。
        for _ in 0..=crate::player::TIMESTOP_FRAMES {
            press(&mut w, crate::input::BTN_TIMESTOP);
        }
        assert_eq!(
            w.body.players[0].time_stops, 1,
            "全程按住只应扣一次资源——电平误判成新触发的话这里会变 0"
        );
        assert_eq!(
            w.body.freeze_left[0], 0,
            "窗口早已跑完，不应被电平误判重新点燃"
        );
    }

    /// 与上条互补：真的松开一帧再按 = 合法的第二次触发，必须照常发动——否则一个"矫枉
    /// 过正、永不二次触发"的实现（比如把 `prev_input` 死锁成恒等于 `input`）也能骗过
    /// 上一条。真沿检测的判别面必须两头都占：假沿要挡、真沿要过。
    #[test]
    fn genuine_second_press_after_release_fires_again() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 2;
        press(&mut w, crate::input::BTN_TIMESTOP); // 第一次触发
        assert_eq!(w.body.players[0].time_stops, 1);

        // 松手,跑完整个窗口——freeze_left 与 prev_input 都归零/清位。
        for _ in 0..crate::player::TIMESTOP_FRAMES {
            let f = w.frame();
            crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(f));
        }
        assert_eq!(w.body.freeze_left[0], 0, "窗口应已自然跑完");

        press(&mut w, crate::input::BTN_TIMESTOP); // 真正的第二次按下（新的上升沿）
        assert_eq!(
            w.body.players[0].time_stops, 0,
            "合法的第二次按下必须照常发动"
        );
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
    }

    /// 复审 round 2 Important 补漏：上面两条一条按住到窗口尽头、一条等整窗跑完再按，
    /// 都没有覆盖"窗口**内部**松手重按"这条路径——`pressed_edge` 判定为真（是货真价实
    /// 的新上升沿）、但 `freeze_left[0]` 仍非零（时停还没解除）。这正是
    /// `freeze_left[0] != 0` 那条门禁唯一管的场景（裁定 #6："时停中再按 = no-op 且不扣
    /// 资源"），删掉它整套测试此前竟然照样绿——因为前两条各自绕开了这条路径。
    ///
    /// 判别力：起始 2 点资源，触发后松手一帧、再跑到窗口正中（约第 90 帧），此时
    /// `freeze_left[0]` 应仍在倒数（非零）；此刻真按一次（新的沿）必须：①不扣资源
    /// （仍是 1）；②不刷新倒计时（继续往下数，不跳回 `TIMESTOP_FRAMES`）。
    #[test]
    fn genuine_press_inside_the_window_is_still_a_free_noop() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].time_stops = 2;
        press(&mut w, crate::input::BTN_TIMESTOP); // frame 0：触发
        assert_eq!(w.body.players[0].time_stops, 1);

        // 松手一帧,再跑到窗口正中（触发后共 89 帧：freeze_left 180→91）。
        let f = w.frame();
        crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(f));
        for _ in 0..88 {
            let f = w.frame();
            crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(f));
        }
        let left_before = w.body.freeze_left[0];
        assert!(
            left_before > 0 && left_before < crate::player::TIMESTOP_FRAMES,
            "应仍在窗口中段倒数（约第 90 帧附近），既未解除也未被这条测试自己撞上边界"
        );

        press(&mut w, crate::input::BTN_TIMESTOP); // 窗口内的一次真沿（松手后重按）
        assert_eq!(
            w.body.players[0].time_stops, 1,
            "时停中再按（即便是货真价实的新沿）也不得扣资源——裁定 #6"
        );
        assert!(
            w.body.freeze_left[0] < left_before,
            "不得刷新倒计时——应继续倒数而非跳回 TIMESTOP_FRAMES"
        );
    }

    // ── bomb 自机入口 + deathbomb（自机能力刀 Task 8）─────────────────────

    /// 自定义表跑一帧（`press` 的兄弟）：需要非 v0 的 `BombCfg` 时用它。
    fn press_with(w: &mut crate::step::World, tables: &crate::tables::WorldTables, buttons: u32) {
        let mut input = crate::input::InputFrame::empty(w.frame());
        input.actions[0].buttons = buttons;
        crate::step::step(w, tables, &crate::ecl::image::EclImage::empty(), &input);
    }

    /// ①②成对：窗口内能救、窗口外救不了。**只写①的话"任何时候 bomb 都能救"照样绿。**
    #[test]
    fn deathbomb_inside_the_window_revives_without_costing_a_life() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(
            w.body.players[0].life_state,
            crate::player::LIFE_ALIVE,
            "该复活"
        );
        assert_eq!(w.body.players[0].lives, lives0, "决死救人**不扣命**");
        assert_eq!(w.body.players[0].state_timer, 0, "窗口计时该清零");
        assert_eq!(w.body.players[0].bombs, 0, "扣一颗 bomb");
    }

    /// ② 窗口已耗尽（`commit_death` 跑过、命已扣）之后再 bomb：救不回来，且**不退款**。
    /// 与①成对才有判别力——单独看①，"任何时候 bomb 都能复活"的错实现照样绿。
    #[test]
    fn bomb_after_the_window_closed_cannot_undo_the_death() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0); // 窗口耗尽 → commit_death
        assert_eq!(w.body.players[0].lives, lives0 - 1, "已经扣命");
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "救不回来，命不会退");
        assert_eq!(
            w.body.players[0].bombs, 1,
            "RESPAWNING 期间根本发动不了，bomb 也不该被扣"
        );
    }

    /// ③ 伤害圆判别式：圈**内**敌掉血、圈**外**敌不掉血。
    /// 圆心重合式的摆法测不出半径映射（CLAUDE.md 点名的 M0-7 教训）。
    #[test]
    fn bomb_damage_field_hits_only_enemies_inside_its_radius() {
        use crate::math::Fx;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.players[0].bombs = 1;
        let near = crate::world::test_support::spawn_enemy(&mut w, 0, 240, 1000); // 距 40 < 120
        let far = crate::world::test_support::spawn_enemy(&mut w, 0, 40, 1000); // 距 160 > 120+16
        let (ni, fi) = (
            w.body.enemies.get(near).unwrap(),
            w.body.enemies.get(far).unwrap(),
        );
        let (nhp, fhp) = (w.body.enemies.hp[ni], w.body.enemies.hp[fi]);
        press(&mut w, crate::input::BTN_BOMB);
        press(&mut w, 0);
        assert!(w.body.enemies.hp[ni] < nhp, "圈内敌必须掉血");
        assert_eq!(w.body.enemies.hp[fi], fhp, "圈外敌不得掉血");
    }

    /// ④ 伤害圆**不跟随**：起爆后把自机挪走，圆心不动（裁定 #10 的后半句）。
    #[test]
    fn bomb_damage_field_does_not_follow_the_player() {
        use crate::math::Fx;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.players[0].bombs = 1;
        press(&mut w, crate::input::BTN_BOMB);
        let f = w
            .body
            .fields
            .iter_alive()
            .find(|&i| w.body.fields.flags[i] & crate::field::FIELD_DAMAGE != 0)
            .expect("应铺了伤害 field");
        let (fx, fy) = (w.body.fields.x[f], w.body.fields.y[f]);
        assert_eq!(
            (fx, fy),
            (Fx::ZERO, Fx::from_int(200)),
            "圆心 = 起爆当帧的自机位（PlayerAtCast）"
        );
        w.body.players[0].x = Fx::from_int(150); // 把自机挪走
        press(&mut w, 0);
        assert_eq!(
            (w.body.fields.x[f], w.body.fields.y[f]),
            (fx, fy),
            "圆心必须钉在起爆点"
        );
    }

    /// ⑤ 持续消弹：起爆后第 60 帧新发射的弹**也被消掉**。写成 `life = 1` 的话这条当场红，
    /// 而只测起爆当帧的写法对它是瞎的（spec §10.4）。
    #[test]
    fn bomb_clear_field_keeps_clearing_for_its_whole_duration() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        press(&mut w, crate::input::BTN_BOMB);
        for _ in 0..59 {
            press(&mut w, 0);
        }
        crate::world::test_support::bullet_at(&mut w, 0, 200); // 第 60 帧新来的弹
        press(&mut w, 0);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "整段期间新弹也该被消掉"
        );
    }

    /// **按住不放不得连环起爆**（本刀的沿检测判别腿）。默认装备带 3 颗 bomb 且相位 3 每帧
    /// 都跑 `try_bomb`，而 `bomb_phase` 在计时归零那一帧当场清 0 —— 若门禁查的是**电平**
    /// 而非上升沿，第 `frames` 帧（C 组刚把 `bomb_phase` 清 0、A 组紧接着又看见电平 1）
    /// 就会立刻点第二颗，按住不放即可把三颗全烧光。时停那边掩盖不了这个：它默认只有
    /// 1 点资源，烧完就没有第二次可烧。
    #[test]
    fn holding_the_bomb_key_does_not_chain_bomb() {
        let mut w = crate::step::World::new(1);
        let bombs0 = w.body.players[0].bombs;
        assert!(bombs0 >= 3, "默认装备应带 3 颗——本测试的判别力靠它");
        let frames = crate::tables::TABLES_V0.characters[0].bomb.frames;
        // 按住跨过两整段效果时长：电平实现会在第 frames 帧与第 2*frames 帧各续一颗。
        for _ in 0..=(2 * frames + 4) {
            press(&mut w, crate::input::BTN_BOMB);
        }
        assert_eq!(
            w.body.players[0].bombs,
            bombs0 - 1,
            "全程按住只该起爆一次——查电平的话这里会被连烧掉 3 颗"
        );
    }

    /// 与上条互补（沿检测两头都要占）：真的松开、等本段效果跑完再按 = 合法的第二发，
    /// 必须照常起爆。否则"永不二次触发"的矫枉过正实现也能骗过上一条。
    #[test]
    fn genuine_second_bomb_after_release_fires_again() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1);
        let frames = crate::tables::TABLES_V0.characters[0].bomb.frames;
        for _ in 0..frames {
            press(&mut w, 0); // 松手跑完整段
        }
        assert_eq!(w.body.players[0].bomb_phase, 0, "本段效果应已自然结束");
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "新的一次真沿必须照常起爆");
        assert_ne!(w.body.players[0].bomb_phase, 0);
    }

    /// 效果进行中再按（松手后重按 ⇒ 货真价实的新沿）= **no-op 且不扣 bomb**。
    /// 这条是 `bomb_phase != 0` 那条门禁唯一管的场景——上面两条各自绕开了它。
    #[test]
    fn pressing_bomb_while_one_is_active_is_a_free_noop() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, crate::input::BTN_BOMB);
        press(&mut w, 0); // 松手一帧，制造真沿的前提
        let left = w.body.players[0].bomb_timer;
        assert!(left > 0, "应仍在效果段内");
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "效果中再按不得扣 bomb");
        assert!(
            w.body.players[0].bomb_timer < left,
            "也不得刷新计时——应继续倒数"
        );
    }

    /// 计时到点自清：`bomb_timer` 归零那一帧 `bomb_phase` 必须跟着清 0，一帧不多不少
    /// （只测"最终会清"的话，"提前一帧清"或"永不清"都能溜过其中一头）。
    #[test]
    fn bomb_phase_clears_exactly_when_the_timer_runs_out() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let frames = crate::tables::TABLES_V0.characters[0].bomb.frames;
        press(&mut w, crate::input::BTN_BOMB); // 触发帧：相位 3 写 phase=1/timer=frames
        assert_eq!(
            w.body.players[0].bomb_timer, frames,
            "触发当帧不该被自己减掉"
        );
        for k in 1..frames {
            press(&mut w, 0);
            assert_ne!(w.body.players[0].bomb_phase, 0, "第 {k} 帧仍应在效果中");
        }
        press(&mut w, 0);
        assert_eq!(w.body.players[0].bomb_timer, 0);
        assert_eq!(w.body.players[0].bomb_phase, 0, "第 frames 帧必须已结束");
    }

    /// 计时归 **C 组**：时停（`freeze_left[0]`）期间 bomb 不流逝、无敌帧不被浪费
    /// （spec §10.2）。放 A 组的实现在这条上会红——它照样每帧减。
    #[test]
    fn bomb_timer_does_not_tick_while_the_scene_is_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        press(&mut w, crate::input::BTN_BOMB);
        let left = w.body.players[0].bomb_timer;
        w.body.freeze_left[0] = 30; // C 组冻结（玩家时停）
        for _ in 0..10 {
            press(&mut w, 0);
        }
        assert_eq!(
            w.body.players[0].bomb_timer, left,
            "C 组冻结期间 bomb 计时必须停摆"
        );
    }

    /// 无 bomb 时按无效（门禁二）。
    #[test]
    fn bomb_without_stock_does_nothing() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 0;
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_phase, 0);
        assert_eq!(w.body.fields.iter_alive().count(), 0, "不得铺任何作用区");
    }

    /// A 组被冻（ECL 演出进行中）时发不出 bomb —— 与时停同源，是 A 组门禁自动给的。
    #[test]
    fn bomb_is_unavailable_while_the_actor_is_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        w.body.freeze_left = [0, 10];
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_phase, 0, "被定住期间不得发动");
        assert_eq!(w.body.players[0].bombs, 2, "也不得扣 bomb");
    }

    /// `fields` 合法可空（"只给无敌"的 bomb 是一张合法的表）：不 panic、不铺区，
    /// 但资源、状态机与无敌照样走完（`for` 循环零轮次不该顺手把别的也跳过）。
    #[test]
    fn a_bomb_with_no_fields_is_legal_and_still_grants_invulnerability() {
        // `WorldTables` 不 derive `Clone`（表体量大，不该鼓励整表复制）；本仓已有的路是
        // `build_tables_v0()` 现构一份 owned 副本再改字段（`tables.rs` 的 `mod tests` 同款）。
        let mut t = crate::tables::build_tables_v0();
        t.characters[0].bomb.fields = Box::new([]);
        let invuln = t.characters[0].bomb.invuln;
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        press_with(&mut w, &t, crate::input::BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "照样扣一颗");
        assert_ne!(w.body.players[0].bomb_phase, 0, "照样进效果段");
        assert_eq!(w.body.fields.iter_alive().count(), 0, "无区可铺");
        // 触发帧 C 组先于 A 组跑（本文件 `update_players` 固定顺序）：C 组检查 invuln 时
        // 它还是触发前的旧值 0，不满足 `>0` 不会自减；随后 A 组的 `try_bomb` 才把它写成
        // `cfg.invuln`。故触发当帧不会被自己减掉——与 `bomb_timer` 同规（见
        // `bomb_phase_clears_exactly_when_the_timer_runs_out` 的"触发当帧不该被自己减掉"）。
        assert_eq!(
            w.body.players[0].invuln, invuln,
            "无敌帧照样给，且触发帧不被自减"
        );
    }

    /// 起爆当帧全屏吸道具（`attract_items = true` 的接线腿）。
    #[test]
    fn bomb_attracts_every_loose_item_on_cast() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let h = w.body.drop_item(
            crate::math::Fx::ZERO,
            crate::math::Fx::from_int(60),
            crate::items::ITEM_POWER,
            &crate::tables::TABLES_V0,
        );
        let ii = w.body.items.get(h).unwrap();
        assert_eq!(
            w.body.items.magnet_to[ii],
            crate::items::MAGNET_NONE,
            "前提：起爆前未上锁"
        );
        press(&mut w, crate::input::BTN_BOMB);
        assert_eq!(w.body.items.magnet_to[ii], 0, "起爆当帧应全场上锁到自机 0");
    }

    /// bomb 起爆 ⇒ 符卡不予收卡。`spell.rs` 的资格轮询（`settle_spells` 步 1）一直写着
    /// `bomb_phase != 0 ⇒ capture_ok = 0`，但在本刀之前**没有任何东西会设 `bomb_phase`**
    /// ——这条测的是那根接线终于通了，不是符卡机构自身（故放在 player.rs 而非 spell.rs）。
    #[test]
    fn bombing_voids_the_spell_capture() {
        let mut w = crate::step::World::new(1);
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        // 实参序以 `world.rs` 的签名为准：
        // (slot, boss, spell_id, time_limit, bonus0, flags, hp_threshold)。
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        assert_ne!(w.body.spells[0].capture_ok, 0, "开卡时资格应在");
        w.body.players[0].bombs = 1;
        press(&mut w, crate::input::BTN_BOMB);
        assert_ne!(w.body.players[0].bomb_phase, 0, "前提：bomb 真的起爆了");
        assert_eq!(w.body.spells[0].capture_ok, 0, "起爆后本卡不予收卡");
    }
}
