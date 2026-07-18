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
    pub(crate) fn decode_input(&mut self, input: &crate::input::InputFrame) {
        self.phase_enter(super::PH_DECODE);
        for i in 0..crate::MAX_PLAYERS {
            self.players[i].input = input.actions[i].buttons;
        }
    }
    pub(crate) fn update_players(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_PLAYERS);
        for i in 0..crate::MAX_PLAYERS {
            // 生死状态机计时（A4 相位 3 职责）
            match self.players[i].life_state {
                LIFE_ABSENT | LIFE_GAMEOVER => continue,
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
            // commit_death 可能刚把 lives 耗尽置 GAMEOVER → 再判一次跳过移动/发弹
            if self.players[i].life_state == LIFE_GAMEOVER {
                continue;
            }
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
        let shooters = cfg.shot.sets[tier][focus];
        let option_pos = cfg.shot.option_pos[tier];
        let (px, py) = (self.players[i].x, self.players[i].y);

        for shooter in shooters {
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

    /// focus 索引到位（M0-17 T4）：`BTN_SLOW` 持下解释器读 `sets[tier][1]`——v0 两焦点槽
    /// 共享同一列表（`tables.rs::tables_v0_shape` 已用 `ptr::eq` 钉死内容层同源），本测试钉
    /// 解释器*真的*用 focus=1 索引且行为等价（同弹数）；内容差异化留后补（spec 拍板 5）。
    #[test]
    fn focus_indexes_focused_set() {
        use crate::input::{BTN_SHOT, BTN_SLOW};

        let mut w = crate::step::World::new(1);
        w.body.players[0].power = 250; // tier2：两路，便于与 tier0 单路区分
        w.body.players[0].input = BTN_SHOT | BTN_SLOW;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_PLAYERS;
        }
        w.body.update_players(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.shots.iter_alive().count(),
            2,
            "focus=1 读到同一 tier2 两路列表（v0 共享内容）"
        );
    }
}
