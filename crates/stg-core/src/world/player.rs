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
    HIGH_SPEED, INV_SQRT2, LIFE_ABSENT, LIFE_ALIVE, LIFE_DEATHWINDOW, LIFE_GAMEOVER,
    LIFE_RESPAWNING, LOW_SPEED, RESPAWN_INVULN, SHOT_CD_FRAMES, SHOT_DAMAGE, SHOT_RADIUS,
    SHOT_SPEED,
};
use crate::shots::ShotInit;

impl WorldBody {
    pub(crate) fn decode_input(&mut self, input: &crate::input::InputFrame) {
        self.phase_enter(super::PH_DECODE);
        for i in 0..crate::MAX_PLAYERS {
            self.players[i].input = input.actions[i].buttons;
        }
    }
    pub(crate) fn update_players(&mut self) {
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
            self.move_player(i);
            // 角色模块静态分发点（A8"shottype 类似物"）：现仅 character 0，将来各角色一臂。
            #[allow(clippy::single_match)]
            match self.players[i].character_id {
                0 => self.char0_update_shot(i),
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

    /// 移动（东方手感：方向 + 低速 + 对角归一 + 场界钳制）。
    fn move_player(&mut self, i: usize) {
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
        let sp = if inp & BTN_SLOW != 0 {
            LOW_SPEED
        } else {
            HIGH_SPEED
        };
        let axis = if dx != 0 && dy != 0 {
            sp * INV_SQRT2
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

    /// character-0 火力（"shottype 类似物"）：SHOT 按下且 CD 到 → 发一发直线上飞弹。
    fn char0_update_shot(&mut self, i: usize) {
        if self.players[i].shot_cd > 0 {
            self.players[i].shot_cd -= 1;
            return;
        }
        if self.players[i].input & crate::input::BTN_SHOT != 0 {
            let (px, py) = (self.players[i].x, self.players[i].y);
            self.create_player_shot(ShotInit {
                x: px,
                y: py,
                vx: Fx::ZERO,
                vy: -SHOT_SPEED, // 上飞
                damage: SHOT_DAMAGE,
                radius: SHOT_RADIUS,
                sprite: 0,
                owner: i as u8,
                flags: 0,
            });
            self.players[i].shot_cd = SHOT_CD_FRAMES;
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
            crate::step::step(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_RESPAWNING);
        assert_eq!(w.body.players[0].lives, lives0 - 1);
        assert!(w.body.players[0].invuln > 0);
        // 再跑够无敌帧 → Alive
        for _ in 0..crate::player::RESPAWN_INVULN {
            crate::step::step(&mut w, &InputFrame::empty(0));
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
            crate::step::step(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_GAMEOVER);
        assert_eq!(w.body.players[0].lives, 0);

        // GAMEOVER 后：给足输入也不该动、不该发弹
        let mut f = InputFrame::empty(100);
        f.actions[0].buttons = BTN_RIGHT | BTN_SHOT;
        for _ in 0..10 {
            crate::step::step(&mut w, &f);
        }
        assert_eq!(w.body.players[0].x, x0, "GAMEOVER 后不该移动");
        assert_eq!(w.body.shots.iter_alive().count(), 0, "GAMEOVER 后不该发弹");
    }
}
