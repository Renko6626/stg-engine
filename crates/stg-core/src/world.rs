//! 世界本体（stg_core::world）—— WorldBody 字段 + `pub(crate)` 相位函数 + 写 API + PhaseGuard。
//! 最小切片（M0-4）：无 ECL/玩家/碰撞；3 输出缓冲随各自生产者再加。
//! **构造只走 `step::World::new`（堆零初始化）**——WorldBody 无 `new()`，避免 ~450KB 栈临时量。

use crate::bullets::{BulletHandle, BulletInit, BulletPool};
use crate::enemy::{ENEMY_DYING, EnemyHandle, EnemyInit, EnemyPool};
use crate::events::{EVENTS_CAP, Event, HITS_CAP, Hit};
use crate::math::Fx;
use crate::player::PlayerState;
use crate::rng::Pcg32;
use crate::shots::{ShotHandle, ShotInit, ShotPool};

// ── 常量：池 id / 错误码 / 场界（D7 中轴原点，384×448 + 越界边距）──────────
pub const POOL_BULLET: usize = 0;
pub const POOL_SHOT: usize = 1;
pub const POOL_ENEMY: usize = 2;
pub const STATUS_OK: u16 = 0;
pub const STATUS_POOL_FULL: u16 = 1;

const FIELD_HALF_W: i32 = 192; // x ∈ [-192, 192]
const FIELD_HEIGHT: i32 = 448; // y ∈ [0, 448]
const OOB_MARGIN: i32 = 64; // 越界回收边距

// ── 相位索引（A4 v2，0-based；PhaseGuard 押运）───────────────────────────
pub(crate) const NUM_PHASES: u8 = 11;
pub(crate) const PH_BEGIN: u8 = 0;
pub(crate) const PH_DECODE: u8 = 1;
pub(crate) const PH_DIRECTOR: u8 = 2;
pub(crate) const PH_PLAYERS: u8 = 3;
pub(crate) const PH_XFORM: u8 = 4;
pub(crate) const PH_INTEGRATE: u8 = 5;
pub(crate) const PH_COLLIDE: u8 = 6;
pub(crate) const PH_SETTLE: u8 = 7;
pub(crate) const PH_ECL_HOOK: u8 = 8;
pub(crate) const PH_CLEANUP: u8 = 9;
pub(crate) const PH_ADVANCE: u8 = 10;

/// 播种流选择（PCG32 seq）；固定进 World 身份。
pub(crate) const RNG_SEQ: u64 = 0xda3e_39cb_94b9_5bdb;

/// 诊断计数器（P4；**参与校验和**——两机必须丢得一样多）。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum)]
pub struct DiagCounters {
    pub pool_full: [u32; 8], // 按池 id
    pub contract_viol: u32,
    pub hits_overflow: u32,   // hits 满丢弃计数（P4-a）
    pub events_overflow: u32, // events 满丢弃计数（P4-a）
}

/// 世界本体（最小切片）。构造走 `step::World::new`（堆零初始化 + 播种 rng）。
#[repr(C)]
#[derive(crate::checksum::Checksum)]
pub struct WorldBody {
    pub frame: u32,
    pub rng: Pcg32,
    pub bullets: BulletPool,
    pub players: [PlayerState; crate::MAX_PLAYERS],
    pub shots: ShotPool,
    pub enemies: EnemyPool,
    #[checksum(skip = "纯输出缓冲，帧内私有，重演确定性再生（A5）")]
    pub(crate) hits: [Hit; HITS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 hits 一并 skip（A5）")]
    pub(crate) hits_len: u16,
    #[checksum(skip = "纯输出缓冲，相位 8/表现层只读，重演确定性再生（A5）")]
    pub events: [Event; EVENTS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 events 一并 skip（A5）")]
    pub events_len: u16,
    pub diag: DiagCounters,
    pub last_status: u16,
    #[cfg(debug_assertions)]
    #[checksum(skip = "debug-only 时序护栏")]
    pub(crate) phase_guard: u8,
}

impl WorldBody {
    /// PhaseGuard：debug 断言相位保序，乱序 panic；release 零成本。
    #[inline]
    pub(crate) fn phase_enter(&mut self, p: u8) {
        #[cfg(debug_assertions)]
        {
            debug_assert_eq!(self.phase_guard, p, "step 相位乱序：期望此相 {p}");
            self.phase_guard = (p + 1) % NUM_PHASES;
        }
        let _ = p;
    }

    // ── 写 API（P1：调用方只走这里，不摸池内存）──────────────────────────
    /// 创建一颗弹（P4-a：池满 → NULL + 诊断计数 + last_status）。
    pub fn create_bullet(&mut self, init: BulletInit) -> BulletHandle {
        match self.bullets.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_BULLET] = self.diag.pool_full[POOL_BULLET].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                BulletHandle::NULL
            }
        }
    }

    /// 创建一发自机弹（P4-a：池满 → NULL + 诊断计数 + last_status）。
    pub fn create_player_shot(&mut self, init: ShotInit) -> ShotHandle {
        match self.shots.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_SHOT] = self.diag.pool_full[POOL_SHOT].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                ShotHandle::NULL
            }
        }
    }

    /// 创建一个敌人（P4-a：池满 → NULL + 诊断计数 + last_status）。
    pub fn create_enemy(&mut self, init: EnemyInit) -> EnemyHandle {
        match self.enemies.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_ENEMY] = self.diag.pool_full[POOL_ENEMY].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                EnemyHandle::NULL
            }
        }
    }

    /// 收集一条碰撞命中（P4-a：满则停收 + 计数，不 panic）。
    pub(crate) fn push_hit(&mut self, row: u8, active: u16, passive: u16) {
        if (self.hits_len as usize) < HITS_CAP {
            self.hits[self.hits_len as usize] = Hit {
                row,
                active,
                passive,
            };
            self.hits_len += 1;
        } else {
            self.diag.hits_overflow = self.diag.hits_overflow.wrapping_add(1);
        }
    }

    /// 产出一条世界大事记（P4-a：满则丢弃 + 计数，不 panic）。
    /// 生产端接入 settle/相位 3 是 Task 3/4/5；本任务仅测试直调，故暂 allow(dead_code)（届时可删）。
    #[allow(dead_code)]
    pub(crate) fn push_event(&mut self, ev: Event) {
        if (self.events_len as usize) < EVENTS_CAP {
            self.events[self.events_len as usize] = ev;
            self.events_len += 1;
        } else {
            self.diag.events_overflow = self.diag.events_overflow.wrapping_add(1);
        }
    }

    /// 越界判定（含边距）。
    fn out_of_bounds(x: Fx, y: Fx) -> bool {
        let xi = x.to_int_floor();
        let yi = y.to_int_floor();
        !(-FIELD_HALF_W - OOB_MARGIN..=FIELD_HALF_W + OOB_MARGIN).contains(&xi)
            || !(-OOB_MARGIN..=FIELD_HEIGHT + OOB_MARGIN).contains(&yi)
    }

    // ── 相位函数（pub(crate)，每个先 phase_enter 保序）────────────────────
    pub(crate) fn begin(&mut self) {
        self.phase_enter(PH_BEGIN);
        self.hits_len = 0;
        self.events_len = 0;
    }
    pub(crate) fn decode_input(&mut self, input: &crate::input::InputFrame) {
        self.phase_enter(PH_DECODE);
        for i in 0..crate::MAX_PLAYERS {
            self.players[i].input = input.actions[i].buttons;
        }
    }
    pub(crate) fn update_players(&mut self) {
        self.phase_enter(PH_PLAYERS);
        for i in 0..crate::MAX_PLAYERS {
            if self.players[i].life_state == crate::player::LIFE_ABSENT {
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

    /// 移动（东方手感：方向 + 低速 + 对角归一 + 场界钳制）。
    fn move_player(&mut self, i: usize) {
        use crate::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SLOW, BTN_UP};
        use crate::player::{HIGH_SPEED, INV_SQRT2, LOW_SPEED};
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
            Fx::from_int(-FIELD_HALF_W).raw(),
            Fx::from_int(FIELD_HALF_W).raw(),
        ));
        p.y = Fx::from_raw(p.y.raw().clamp(0, Fx::from_int(FIELD_HEIGHT).raw()));
    }

    /// character-0 火力（"shottype 类似物"）：SHOT 按下且 CD 到 → 发一发直线上飞弹。
    fn char0_update_shot(&mut self, i: usize) {
        use crate::player::{SHOT_CD_FRAMES, SHOT_DAMAGE, SHOT_RADIUS, SHOT_SPEED};
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
    pub(crate) fn run_transforms(&mut self) {
        self.phase_enter(PH_XFORM); // stub：无变换段池
    }
    pub(crate) fn integrate(&mut self) {
        self.phase_enter(PH_INTEGRATE);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.bullets.delay[i] > 0 {
                    self.bullets.delay[i] -= 1; // delay 期不动
                    continue;
                }
                self.bullets.x[i] = self.bullets.x[i] + self.bullets.vx[i];
                self.bullets.y[i] = self.bullets.y[i] + self.bullets.vy[i];
                if self.bullets.life[i] != 0xFFFF && self.bullets.life[i] > 0 {
                    self.bullets.life[i] -= 1;
                }
            }
        }
        // 自机弹：pos += vel（无 delay/life）
        let nw = self.shots.alive.len();
        for w in 0..nw {
            let mut bits = self.shots.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.shots.x[i] = self.shots.x[i] + self.shots.vx[i];
                self.shots.y[i] = self.shots.y[i] + self.shots.vy[i];
            }
        }
        // 敌人：pos += vel（move_to 插值器延后，mv_* 惰性）+ 计时器 tick
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.enemies.x[i] = self.enemies.x[i] + self.enemies.vx[i];
                self.enemies.y[i] = self.enemies.y[i] + self.enemies.vy[i];
                if self.enemies.invuln[i] > 0 {
                    self.enemies.invuln[i] -= 1;
                }
                if self.enemies.hit_flash[i] > 0 {
                    self.enemies.hit_flash[i] -= 1;
                }
            }
        }
    }
    pub(crate) fn collide(&mut self) {
        self.phase_enter(PH_COLLIDE);
        self.collide_bullets_player(); // 行 1/2：敌弹 × 自机
        self.collide_body_player(); // 行 3：敌体 × 自机
        self.collide_shot_enemy(); // 行 4：自机弹 × 敌人
    }

    /// 行 1（hit）+ 行 2（graze）：敌弹 × 自机。一次 len_sq 复用两半径。
    fn collide_bullets_player(&mut self) {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        use crate::math::geom::len_sq;
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE
                || self.players[p].invuln != 0
            {
                continue; // 门禁：只 Alive 且非无敌参与
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let graze_r = self.players[p].graze_radius;
            let nw = self.bullets.alive.len();
            for w in 0..nw {
                let mut bits = self.bullets.alive[w];
                while bits != 0 {
                    let b = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if self.bullets.delay[b] > 0 {
                        continue; // delay 弹不参与
                    }
                    let dx = self.bullets.x[b] - px;
                    let dy = self.bullets.y[b] - py;
                    let d2 = len_sq(dx, dy);
                    let br = self.bullets.radius[b];
                    let graze_sum = (br + graze_r).raw() as i64;
                    if d2 <= graze_sum * graze_sum {
                        self.push_hit(ROW_BULLET_PLAYER_GRAZE, b as u16, p as u16);
                        let hit_sum = (br + hit_r).raw() as i64;
                        if d2 <= hit_sum * hit_sum {
                            self.push_hit(ROW_BULLET_PLAYER_HIT, b as u16, p as u16);
                        }
                    }
                }
            }
        }
    }
    /// 行 3：敌体（enemy.radius）× 自机 hit_radius。
    fn collide_body_player(&mut self) {
        use crate::events::ROW_BODY_PLAYER_HIT;
        use crate::math::geom::len_sq;
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE
                || self.players[p].invuln != 0
            {
                continue;
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let nw = self.enemies.alive.len();
            for w in 0..nw {
                let mut bits = self.enemies.alive[w];
                while bits != 0 {
                    let e = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let dx = self.enemies.x[e] - px;
                    let dy = self.enemies.y[e] - py;
                    let d2 = len_sq(dx, dy);
                    let sum = (self.enemies.radius[e] + hit_r).raw() as i64;
                    if d2 <= sum * sum {
                        self.push_hit(ROW_BODY_PLAYER_HIT, e as u16, p as u16);
                    }
                }
            }
        }
    }

    /// 行 4：自机弹（shot.radius）× 敌人 hurtbox（受击圈）。
    /// 嵌套固定：shot 外层、enemy 内层（升序）→ settle 扣血序确定。无敌帧过滤留给 settle。
    fn collide_shot_enemy(&mut self) {
        use crate::events::ROW_SHOT_ENEMY;
        use crate::math::geom::len_sq;
        let ne = self.enemies.alive.len();
        let nws = self.shots.alive.len();
        for sw in 0..nws {
            let mut sbits = self.shots.alive[sw];
            while sbits != 0 {
                let s = sw * 64 + sbits.trailing_zeros() as usize;
                sbits &= sbits - 1;
                let (sx, sy) = (self.shots.x[s], self.shots.y[s]);
                let sr = self.shots.radius[s];
                for ew in 0..ne {
                    let mut ebits = self.enemies.alive[ew];
                    while ebits != 0 {
                        let e = ew * 64 + ebits.trailing_zeros() as usize;
                        ebits &= ebits - 1;
                        let dx = self.enemies.x[e] - sx;
                        let dy = self.enemies.y[e] - sy;
                        let d2 = len_sq(dx, dy);
                        let sum = (sr + self.enemies.hurtbox[e]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_SHOT_ENEMY, s as u16, e as u16);
                        }
                    }
                }
            }
        }
    }
    pub(crate) fn settle(&mut self) {
        self.phase_enter(PH_SETTLE); // stub
    }
    pub(crate) fn cleanup(&mut self) {
        self.phase_enter(PH_CLEANUP);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = (self.bullets.life[i] != 0xFFFF && self.bullets.life[i] == 0)
                    || Self::out_of_bounds(self.bullets.x[i], self.bullets.y[i]);
                if dead {
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
                    || Self::out_of_bounds(self.enemies.x[i], self.enemies.y[i]);
                if dead {
                    self.enemies.free_index(i);
                }
            }
        }
    }
    pub(crate) fn advance(&mut self) {
        self.phase_enter(PH_ADVANCE);
        self.frame += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn hits_push_clear_and_overflow() {
        use crate::events::HITS_CAP;
        let mut w = crate::step::World::new(1);
        w.body.push_hit(1, 3, 0);
        w.body.push_hit(2, 4, 0);
        assert_eq!(w.body.hits_len, 2);
        // 溢出：填满后再推 → 停收 + 计数，不 panic
        w.body.hits_len = HITS_CAP as u16;
        w.body.push_hit(1, 0, 0);
        assert_eq!(w.body.hits_len, HITS_CAP as u16); // 未增
        assert_eq!(w.body.diag.hits_overflow, 1);
        // begin 清空
        w.body.begin();
        assert_eq!(w.body.hits_len, 0);
        assert_eq!(w.body.events_len, 0);
    }

    #[test]
    fn events_push_records_fact() {
        use crate::events::{EVT_PLAYER_DIED, Event};
        let mut w = crate::step::World::new(1);
        w.body.push_event(Event {
            kind: EVT_PLAYER_DIED,
            a_index: 0,
            a_gen: 0,
            x: Fx::ZERO,
            y: Fx::from_int(384),
            data: [2, 0],
        });
        assert_eq!(w.body.events_len, 1);
        assert_eq!(w.body.events[0].kind, EVT_PLAYER_DIED);
    }

    // 造一颗停在 (x,y) 的哑弹（半径 2）。
    fn bullet_at(w: &mut crate::step::World, x: i32, y: i32) {
        w.body.create_bullet(crate::bullets::BulletInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
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
    }

    #[test]
    fn collide_bullet_on_player_collects_hit_and_graze() {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        let mut w = crate::step::World::new(1);
        // 自机在 (0,384)，hit_radius=2.5、graze_radius=16。弹压在自机身上 → 中弹+擦弹都收。
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        let hit = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_HIT)
            .count();
        let graze = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_GRAZE)
            .count();
        assert_eq!(hit, 1);
        assert_eq!(graze, 1);
    }

    #[test]
    fn collide_near_bullet_grazes_only() {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 10, 384); // 距 10px：在 graze 圈(≈18)内、hit 圈(≈4.5)外
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        let hit = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_HIT)
            .count();
        let graze = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_GRAZE)
            .count();
        assert_eq!(hit, 0);
        assert_eq!(graze, 1);
    }

    #[test]
    fn collide_skips_invuln_player() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        w.body.players[0].invuln = 60; // 无敌 → 不参与
        bullet_at(&mut w, 0, 384);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(w.body.hits_len, 0);
    }

    #[test]
    fn collide_skips_delay_bullet() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        // 把刚造的弹设 delay>0（索引 0）
        w.body.bullets.delay[0] = 5;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(w.body.hits_len, 0);
    }

    #[test]
    fn create_enemy_and_integrate_moves() {
        use crate::enemy::EnemyInit;
        let mut w = crate::step::World::new(1);
        let init = EnemyInit {
            x: Fx::ZERO,
            y: Fx::from_int(50),
            vx: Fx::from_int(1),
            vy: Fx::from_int(2),
            mv_from_x: Fx::ZERO,
            mv_from_y: Fx::ZERO,
            mv_to_x: Fx::ZERO,
            mv_to_y: Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp: 5,
            hp_max: 5,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 100,
        };
        let h = w.body.create_enemy(init);
        assert_ne!(h, crate::enemy::EnemyHandle::NULL);
        crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.x[i], Fx::from_int(1)); // 0+1
        assert_eq!(w.body.enemies.y[i], Fx::from_int(52)); // 50+2
    }

    fn spawn_enemy(
        w: &mut crate::step::World,
        x: i32,
        y: i32,
        hp: i32,
    ) -> crate::enemy::EnemyHandle {
        w.body.create_enemy(crate::enemy::EnemyInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
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
            hp,
            hp_max: hp,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 100,
        })
    }

    #[test]
    fn collide_enemy_body_on_player() {
        use crate::events::ROW_BODY_PLAYER_HIT;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        spawn_enemy(&mut w, 0, 100, 5); // 敌体 radius 12 + 自机 hit 2.5 → 圆心重合必撞
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        let n = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BODY_PLAYER_HIT)
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn collide_shot_on_enemy() {
        use crate::events::ROW_SHOT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 5);
        let ei = w.body.enemies.get(e).unwrap();
        // 造一发压在敌人身上的自机弹
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
        let hits: Vec<_> = (0..w.body.hits_len as usize)
            .map(|k| w.body.hits[k])
            .filter(|h| h.row == ROW_SHOT_ENEMY)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].active, 0); // shot 索引
        assert_eq!(hits[0].passive as usize, ei); // enemy 索引
    }
}
