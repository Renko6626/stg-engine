//! 世界本体（stg_core::world）—— WorldBody 字段 + `pub(crate)` 相位函数 + 写 API + PhaseGuard。
//! M0-7：collide（D8 行1/2/3/4，圆-圆平方距离，只收集）+ settle（D9 三趟）+ 生死状态机 + EnemyPool 已落。
//! M0-8：FieldPool（通用圆形作用区，静止哑原语）+ 行6 消弹/行7 伤敌 + settle 趟一（标记 + 聚合 FieldCleared）已落。
//! **构造只走 `step::World::new`（堆零初始化）**——WorldBody 无 `new()`，避免 ~450KB 栈临时量。

use crate::bullets::{BulletHandle, BulletInit, BulletPool};
use crate::enemy::{ENEMY_DYING, EnemyHandle, EnemyInit, EnemyPool};
use crate::events::{EVENTS_CAP, Event, HITS_CAP, Hit};
use crate::field::{FieldHandle, FieldInit, FieldPool};
use crate::math::Fx;
use crate::player::PlayerState;
use crate::rng::Pcg32;
use crate::shots::{ShotHandle, ShotInit, ShotPool};

// ── 常量：池 id / 错误码 / 场界（D7 中轴原点，384×448 + 越界边距）──────────
pub const POOL_BULLET: usize = 0;
pub const POOL_SHOT: usize = 1;
pub const POOL_ENEMY: usize = 2;
pub const POOL_FIELD: usize = 3;
pub const STATUS_OK: u16 = 0;
pub const STATUS_POOL_FULL: u16 = 1;

/// 所有实体判定半径的写 API 上限（P4-b）。
///
/// 任意两半径之和 ≤ 2×1024 = 2048 ≪ `Fx` 上限 32767.99998 —— 只要六行碰撞两侧都 ≤1024，
/// `(r_active + r_passive)` **裸 i32 Fx 加法**（`Fx::Add`，debug panic / release wrap）就不溢出。
/// 只钳一侧不构成证明：`i32::MAX - 1024*65536` ⇒ 被动半径 > ~31744px 时和仍会溢出（负半径
/// 同理，和变负、平方后仍为正，判定行为诡异）。但"两侧都 ≤1024"这句话背后是**两条强度不同的
/// 保证**，不要读成"四个写 API 覆盖了全部六行"：
///
/// - **池侧**（六行里除自机半径外的全部操作数，即弹/敌/自机弹/field 的 radius/hurtbox）：
///   经 `create_bullet`/`create_enemy`（radius + hurtbox）/`create_player_shot`/`create_field`
///   四个写 API 双边钳入 `[0, MAX_ENTITY_RADIUS]`。这四个池的 SoA 数组是 `pub(crate)`，
///   "只能走写 API"是类型系统**可强制**的纪律，不是约定。
/// - **自机侧**（行 1/2/3 的被动操作数，`PlayerState::hit_radius`/`graze_radius`）：**不经任何
///   写 API**——由 `PlayerState::spawn`（`crate::player`）直接从引擎常量赋值，上限由
///   `player.rs` 里的编译期断言钉死（`HIT_RADIUS`/`GRAZE_RADIUS` ≤ `MAX_ENTITY_RADIUS`）。但
///   `WorldBody.players` 与 `PlayerState` 的字段目前都是 `pub`，任何持 `&mut World` 的上层
///   （今天是 stg-harness，将来是 stg-godot/stg-py）都能绕过 `spawn` 直接写这两个字段——这是
///   **前提**，不是强制。安全性目前只因"除 spawn 外无人写它"成立；收紧可见性（或改走访问器）
///   留待后续。
pub const MAX_ENTITY_RADIUS: Fx = Fx::from_int(1024);

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
    pub fields: FieldPool,
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

    /// 把半径钳入 `[0, MAX_ENTITY_RADIUS]`（P4-b）；返回是否发生了钳制。
    ///
    /// 上界钳住是六行碰撞 `r_active + r_passive` 这个 Fx 加法不溢出的一半证明（另一半在对侧
    /// 调用点也钳）；下界钳到 0 是因为负半径会让"和"变负、平方后却仍为正，判定行为诡异——
    /// `radius = 0` 是良定义的退化点判定，非错误。调用方一次写 API 调用里即便有多个半径字段
    /// 越界，也只应计一次 `contract_viol`（约定：调用方对多个字段的返回值做 `||`，不逐个累加）。
    #[inline]
    fn clamp_radius(r: &mut Fx) -> bool {
        if r.raw() > MAX_ENTITY_RADIUS.raw() {
            *r = MAX_ENTITY_RADIUS;
            true
        } else if r.raw() < 0 {
            *r = Fx::ZERO;
            true
        } else {
            false
        }
    }

    /// 创建一颗弹（P4-a：池满 → NULL + 诊断计数 + last_status；P4-b：radius 双边钳入
    /// `[0, MAX_ENTITY_RADIUS]` + 计数）。
    pub fn create_bullet(&mut self, mut init: BulletInit) -> BulletHandle {
        if Self::clamp_radius(&mut init.radius) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        match self.bullets.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_BULLET] = self.diag.pool_full[POOL_BULLET].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                BulletHandle::NULL
            }
        }
    }

    /// 创建一发自机弹（P4-a：池满 → NULL + 诊断计数 + last_status；P4-b：radius 双边钳入
    /// `[0, MAX_ENTITY_RADIUS]` + 计数）。
    pub fn create_player_shot(&mut self, mut init: ShotInit) -> ShotHandle {
        if Self::clamp_radius(&mut init.radius) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        match self.shots.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_SHOT] = self.diag.pool_full[POOL_SHOT].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                ShotHandle::NULL
            }
        }
    }

    /// 创建一个敌人（P4-a：池满 → NULL + 诊断计数 + last_status；P4-b：`radius`（体碰）与
    /// `hurtbox`（受击）各自双边钳入 `[0, MAX_ENTITY_RADIUS]`；两者同一调用内都越界也只计
    /// 一次 `contract_viol`）。
    pub fn create_enemy(&mut self, mut init: EnemyInit) -> EnemyHandle {
        let viol = Self::clamp_radius(&mut init.radius) | Self::clamp_radius(&mut init.hurtbox);
        if viol {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        match self.enemies.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_ENEMY] = self.diag.pool_full[POOL_ENEMY].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                EnemyHandle::NULL
            }
        }
    }

    /// 创建一个作用区（P4-a：池满 → NULL + 计数；P4-b：radius 双边钳入 `[0, MAX_ENTITY_RADIUS]` + 计数）。
    pub fn create_field(&mut self, mut init: FieldInit) -> FieldHandle {
        // P4-b：调用方违约 → 确定性安全结果。与 create_bullet/create_enemy/create_player_shot
        // 共用同一 MAX_ENTITY_RADIUS——四个写 API 都双边钳，覆盖池侧半径的证明（自机侧半径不
        // 经写 API，另有编译期断言，见 `MAX_ENTITY_RADIUS` 文档的完整两段式推导）。
        if Self::clamp_radius(&mut init.radius) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        match self.fields.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_FIELD] = self.diag.pool_full[POOL_FIELD].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                FieldHandle::NULL
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
    /// 生产端：settle 趟二（敌人致死/自机中弹→窗口）+ update_players 的 commit_death（自机死亡结算）。
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
        use crate::player::{
            LIFE_ABSENT, LIFE_ALIVE, LIFE_DEATHWINDOW, LIFE_GAMEOVER, LIFE_RESPAWNING,
        };
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
        use crate::player::{LIFE_GAMEOVER, LIFE_RESPAWNING, RESPAWN_INVULN};
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
        // 作用区：寿命倒数（照抄弹的模式；life=1 → 本帧减到 0，相位6 仍参与判定，相位9 回收）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] > 0 {
                    self.fields.life[i] -= 1;
                }
            }
        }
    }
    pub(crate) fn collide(&mut self) {
        self.phase_enter(PH_COLLIDE);
        self.collide_bullets_player(); // 行 1/2：敌弹 × 自机
        self.collide_body_player(); // 行 3：敌体 × 自机
        self.collide_shot_enemy(); // 行 4：自机弹 × 敌人
        self.collide_field_bullet(); // 行 6：作用区 × 敌弹（消弹）
        self.collide_field_enemy(); // 行 7：作用区 × 敌人（伤敌）
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
        let nwe = self.enemies.alive.len();
        let nws = self.shots.alive.len();
        for sw in 0..nws {
            let mut sbits = self.shots.alive[sw];
            while sbits != 0 {
                let s = sw * 64 + sbits.trailing_zeros() as usize;
                sbits &= sbits - 1;
                let (sx, sy) = (self.shots.x[s], self.shots.y[s]);
                let sr = self.shots.radius[s];
                for ew in 0..nwe {
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
    /// 行 6：作用区（field.radius）× 敌弹（bullet.radius）→ 消弹。
    /// 能力位在收集前 gate（未开 CLEAR 的 field 整行跳过，省 O(N×M)）。
    fn collide_field_bullet(&mut self) {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_CLEAR_BULLETS;
        use crate::math::geom::len_sq;
        let nwf = self.fields.alive.len();
        let nwb = self.bullets.alive.len();
        for fw in 0..nwf {
            let mut fbits = self.fields.alive[fw];
            while fbits != 0 {
                let f = fw * 64 + fbits.trailing_zeros() as usize;
                fbits &= fbits - 1;
                if self.fields.flags[f] & FIELD_CLEAR_BULLETS == 0 {
                    continue;
                }
                let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
                let fr = self.fields.radius[f];
                for bw in 0..nwb {
                    let mut bbits = self.bullets.alive[bw];
                    while bbits != 0 {
                        let b = bw * 64 + bbits.trailing_zeros() as usize;
                        bbits &= bbits - 1;
                        if self.bullets.delay[b] > 0 {
                            continue; // delay 弹不参与
                        }
                        let dx = self.bullets.x[b] - fx;
                        let dy = self.bullets.y[b] - fy;
                        let d2 = len_sq(dx, dy);
                        let sum = (fr + self.bullets.radius[b]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_FIELD_BULLET, f as u16, b as u16);
                        }
                    }
                }
            }
        }
    }

    /// 行 7：作用区（field.radius）× 敌人 hurtbox（受击圈，与行 4 同）→ 扣血。
    /// **不查敌 invuln**（事件照收、结算时判，与行 4 同规）。
    fn collide_field_enemy(&mut self) {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_DAMAGE;
        use crate::math::geom::len_sq;
        let nwf = self.fields.alive.len();
        let nwe = self.enemies.alive.len();
        for fw in 0..nwf {
            let mut fbits = self.fields.alive[fw];
            while fbits != 0 {
                let f = fw * 64 + fbits.trailing_zeros() as usize;
                fbits &= fbits - 1;
                if self.fields.flags[f] & FIELD_DAMAGE == 0 {
                    continue;
                }
                let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
                let fr = self.fields.radius[f];
                for ew in 0..nwe {
                    let mut ebits = self.enemies.alive[ew];
                    while ebits != 0 {
                        let e = ew * 64 + ebits.trailing_zeros() as usize;
                        ebits &= ebits - 1;
                        let dx = self.enemies.x[e] - fx;
                        let dy = self.enemies.y[e] - fy;
                        let d2 = len_sq(dx, dy);
                        let sum = (fr + self.enemies.hurtbox[e]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_FIELD_ENEMY, f as u16, e as u16);
                        }
                    }
                }
            }
        }
    }

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
        self.phase_enter(PH_SETTLE);
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
    pub(crate) fn cleanup(&mut self) {
        self.phase_enter(PH_CLEANUP);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = (self.bullets.life[i] != 0xFFFF && self.bullets.life[i] == 0)
                    || self.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0
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
        // 作用区：寿命尽回收（不做越界——field 是有意放置的静止圆，非飞行物）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] == 0 {
                    self.fields.free_index(i);
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

    // D8 双半径不对称的判别式测试：行 3（敌体×自机）必须用 enemy.radius（体碰，小），
    // 不能用 enemy.hurtbox（受击，大）。圆心重合（d2=0）没法判别——任何正半径和都会命中；
    // 必须选一个"卡在两个半径和之间"的距离才能让写反的代码露馅，所以这是新增负向测试而非
    // 修改 collide_enemy_body_on_player（那个测试仍保留，用来证明行 3 本身会触发）。
    //
    // 几何：player.hit_radius=2.5，spawn_enemy 固定 radius=12 / hurtbox=16。
    // 轴对齐偏移 15px → d2 = 15² = 225。
    //   正确（radius）：sum = 12+2.5 = 14.5 → 14.5² = 210.25 < 225 → 不命中。
    //   写反（hurtbox）：sum = 16+2.5 = 18.5 → 18.5² = 342.25 > 225 → 命中——测试就会失败。
    #[test]
    fn collide_body_uses_body_radius_not_hurtbox() {
        use crate::events::ROW_BODY_PLAYER_HIT;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        spawn_enemy(&mut w, 15, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        let n = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BODY_PLAYER_HIT)
            .count();
        assert_eq!(n, 0);
    }

    // 几何同上一条判别式思路，但行 4（自机弹×敌人）用 enemy.hurtbox（受击，大），
    // 用轴对齐 18px 偏移即可正向判别（不需要额外负向测试）：
    //   正确（hurtbox）：sum = shot.radius(4)+16 = 20 → 20² = 400 > d2(18²=324) → 命中。
    //   写反（radius）  ：sum = 4+12 = 16 → 16² = 256 < 324 → 不命中——测试就会失败。
    #[test]
    fn collide_shot_on_enemy() {
        use crate::events::ROW_SHOT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 5);
        let ei = w.body.enemies.get(e).unwrap();
        // 自机弹与敌人轴对齐偏移 18px（不再圆心重合，见上方注释的判别式几何）。
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei] + Fx::from_int(18),
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

    fn spawn_field(
        w: &mut crate::step::World,
        x: i32,
        y: i32,
        radius: i32,
        flags: u8,
        life: u16,
    ) -> crate::field::FieldHandle {
        w.body.create_field(crate::field::FieldInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            radius: Fx::from_int(radius),
            dmg_per_frame: 0,
            life,
            owner: 0,
            flags,
        })
    }

    #[test]
    fn create_field_clamps_radius() {
        use crate::field::FIELD_MAX_RADIUS;
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 30000, crate::field::FIELD_CLEAR_BULLETS, 1);
        let i = w.body.fields.get(h).unwrap();
        assert_eq!(w.body.fields.radius[i], FIELD_MAX_RADIUS); // P4-b 钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    #[test]
    fn field_life_one_lives_exactly_one_frame() {
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 20, crate::field::FIELD_CLEAR_BULLETS, 1);
        assert!(w.body.fields.get(h).is_some());
        crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.fields.get(h), None); // 活一帧后 cleanup 回收
    }

    #[test]
    fn field_life_n_survives_n_frames() {
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 20, crate::field::FIELD_CLEAR_BULLETS, 3);
        for _ in 0..2 {
            crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
            assert!(w.body.fields.get(h).is_some()); // 前 2 帧仍在
        }
        crate::step::step(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.fields.get(h), None); // 第 3 帧尽
    }

    #[test]
    fn create_field_clamps_negative_radius_to_zero() {
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, -5, crate::field::FIELD_CLEAR_BULLETS, 1);
        let i = w.body.fields.get(h).unwrap();
        assert_eq!(w.body.fields.radius[i], Fx::ZERO); // P4-b 下界钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    // 造一颗停在原点、半径可指定的哑弹（同 bullet_at，多一个 radius 参数供钳制测试用）。
    fn bullet_with_radius(w: &mut crate::step::World, radius: i32) -> crate::bullets::BulletHandle {
        w.body.create_bullet(crate::bullets::BulletInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: crate::math::Angle::ZERO,
            ang_vel: 0,
            accel: Fx::ZERO,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            sprite: 0,
            radius: Fx::from_int(radius),
            delay: 0,
            life: 0xFFFF,
            flags: 0,
            grazed_by: 0,
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
        })
    }

    #[test]
    fn create_bullet_clamps_radius() {
        let mut w = crate::step::World::new(1);
        let h = bullet_with_radius(&mut w, 30000);
        let i = w.body.bullets.get(h).unwrap();
        assert_eq!(w.body.bullets.radius[i], MAX_ENTITY_RADIUS); // P4-b 钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    #[test]
    fn create_bullet_clamps_negative_radius_to_zero() {
        let mut w = crate::step::World::new(1);
        let h = bullet_with_radius(&mut w, -5);
        let i = w.body.bullets.get(h).unwrap();
        assert_eq!(w.body.bullets.radius[i], Fx::ZERO); // P4-b 下界钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    #[test]
    fn create_enemy_clamps_hurtbox() {
        let mut w = crate::step::World::new(1);
        let h = w.body.create_enemy(crate::enemy::EnemyInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
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
            hurtbox: Fx::from_int(30000), // 受击半径越界，体碰半径不越界
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 0,
        });
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.hurtbox[i], MAX_ENTITY_RADIUS); // P4-b 钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    #[test]
    fn create_player_shot_clamps_radius() {
        let mut w = crate::step::World::new(1);
        let h = w.body.create_player_shot(crate::shots::ShotInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 1,
            radius: Fx::from_int(30000),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        let i = w.body.shots.get(h).unwrap();
        assert_eq!(w.body.shots.radius[i], MAX_ENTITY_RADIUS); // P4-b 钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    #[test]
    fn collide_field_bullet_discriminates_radius_sum() {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_CLEAR_BULLETS;
        // field 半径 20 + 弹半径 2 = 和 22 → 21px 撞、23px 不撞（判别式，非圆心重合）
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 21, 100); // 索引 0：内
        bullet_at(&mut w, 23, 100); // 索引 1：外
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        let hits: Vec<_> = (0..w.body.hits_len as usize)
            .map(|k| w.body.hits[k])
            .filter(|h| h.row == ROW_FIELD_BULLET)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].active, 0); // field 索引
        assert_eq!(hits[0].passive, 0); // 只有 21px 那颗
    }

    #[test]
    fn collide_field_skips_bullets_without_clear_bit() {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_DAMAGE;
        // 只开 DAMAGE 位的 field 压着弹 → 不消弹
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_DAMAGE, 1);
        bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_BULLET)
                .count(),
            0
        );
    }

    #[test]
    fn collide_field_enemy_uses_hurtbox() {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_DAMAGE;
        // field 半径 20 + 敌 hurtbox 16 = 和 36；若误用敌 radius 12 → 和 32
        // 敌人放 34px：正确(≤36)撞；误用 radius(≤32) 则不撞 → 判别式
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_DAMAGE, 1);
        spawn_enemy(&mut w, 34, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_ENEMY)
                .count(),
            1
        );
    }

    #[test]
    fn collide_field_skips_enemy_without_damage_bit() {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_CLEAR_BULLETS;
        // 只开 CLEAR 位的 field 压着敌人 → 不伤敌
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        spawn_enemy(&mut w, 0, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide();
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_ENEMY)
                .count(),
            0
        );
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
