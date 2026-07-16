//! 世界本体（stg_core::world）—— `WorldBody` 字段所有权 + 跨相位共用设施。
//!
//! **模块结构镜像 step 的相位骨架**（P2：相位顺序是宪法，由 `stg_core::step` 独占）。
//! 有分量的相位各占一个子模块，本文件只留字段与共用面：
//!
//! | 相位 | 在哪 |
//! |---|---|
//! | 0 `begin` · 10 `advance` | 本文件（各数行，不值得单开） |
//! | 1 `decode_input` · 3 `update_players` | [`player`] —— 注意 `crate::player` 是 `PlayerState` **数据**模块，本模块是**相位逻辑** |
//! | 4 `run_transforms` | [`transform`] —— D4 游标执行器：瞬时 7 op + wait 门 + END/未知 op 终止 |
//! | 5 `integrate` | [`integrate`] |
//! | 6 `collide` | [`collide`] —— D8 矩阵四类六行，**只收集不改状态** |
//! | 7 `settle` | [`settle`] —— D9 三趟，**唯一改状态者** |
//! | 9 `cleanup` | [`cleanup`] —— 越界/寿命/已清除弹/dying 敌人在此收尸 |
//!
//! 本文件持有：`WorldBody` 结构（字段所有权集中处）· `phase_enter`（PhaseGuard 押运）·
//! 4 个 `create_*` 写 API + `clamp_radius`（P1 边界，P4-b 半径钳制）· `push_hit`/`push_event`
//! （纯输出缓冲的唯一入口）· 场界常量（`FIELD_*`/`OOB_MARGIN`，被 player 与 cleanup 共用）。
//!
//! **构造只走 `step::World::new`（堆零初始化）**——WorldBody 无 `new()`，避免 ~450KB 栈临时量。

use crate::bullets::{BulletHandle, BulletInit, BulletPool};
use crate::enemy::{EnemyHandle, EnemyInit, EnemyPool};
use crate::events::{EVENTS_CAP, Event, HITS_CAP, Hit};
use crate::field::{FieldHandle, FieldInit, FieldPool};
use crate::math::Fx;
use crate::player::PlayerState;
use crate::rng::Pcg32;
use crate::shots::{ShotHandle, ShotInit, ShotPool};

mod cleanup;
mod collide;
mod integrate;
mod motion;
mod player;
mod settle;
mod transform;

// ── 常量：池 id / 错误码 / 场界（D7 中轴原点，384×448 + 越界边距）──────────
pub const POOL_BULLET: usize = 0;
pub const POOL_SHOT: usize = 1;
pub const POOL_ENEMY: usize = 2;
pub const POOL_FIELD: usize = 3;
pub const POOL_XFORM: usize = 4;
pub const POOL_ITEM: usize = 5;
pub const STATUS_OK: u16 = 0;
pub const STATUS_POOL_FULL: u16 = 1;
pub const STATUS_STALE_HANDLE: u16 = 2;
pub const STATUS_BAD_ARGS: u16 = 3;

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

/// 信号黑板通道数（D4 11b）。
pub const SIGNAL_CHANNELS: usize = 8;

pub(crate) const FIELD_HALF_W: i32 = 192; // x ∈ [-192, 192]
pub(crate) const FIELD_HEIGHT: i32 = 448; // y ∈ [0, 448]
pub(crate) const OOB_MARGIN: i32 = 64; // 越界回收边距
#[allow(dead_code)] // 待磁吸相位（后续切片）读取判定 ALIVE 自机是否低于回收线；本切片只搭常量
pub(crate) const POC_LINE_Y: i32 = 128; // 回收线（PoC）：ALIVE 自机 y 低于此线 → 全场道具磁吸

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
    /// 道具池（D7）。与四实体池同级 `pub`——表现层将来要读。
    pub items: crate::items::ItemPool,
    /// 变换段池（D4）。手写 Checksum 全量入校验和（P6）；I7 inline 数组。
    pub(crate) xforms: crate::xform::XformSegPool,
    /// 信号黑板（D4 11b）：每通道存"最后脉冲帧号 + 1"，0 = 从未脉冲（零初始化合法）。
    /// 边沿消费：相位 4 只放行 `signals[ch] == frame + 1` 的停驻弹。
    pub(crate) signals: [u32; SIGNAL_CHANNELS],
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
        init.transform_head = crate::xform::XFORM_NONE; // 哑弹哨兵：调用方伪造段号无效
        match self.bullets.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_BULLET] = self.diag.pool_full[POOL_BULLET].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                BulletHandle::NULL
            }
        }
    }

    /// 创建一颗带变换序列的弹（D4）。序列**拷贝**进弹自有段（尾部清零 = 天然 END）。
    /// P4：radius 双边钳入 `[0, MAX_ENTITY_RADIUS]`（与 `create_bullet` 对称）；坏参
    /// （>16 槽 / 含未知 op / STEP 无扩展槽空间 / easing id ≥ 8 / LOOP target 非边界）
    /// → 整体失败 NULL + BAD_ARGS（宁缺勿哑）；
    /// 先段后弹——段满 → NULL + POOL_FULL(XFORM)；弹池满 → 还段回滚 + POOL_FULL(BULLET)。
    /// `init.transform_head` 恒被本函数覆写（调用方传值无效）。
    pub fn create_bullet_with_xform(
        &mut self,
        mut init: BulletInit,
        xform: &[crate::xform::XformSlot],
    ) -> BulletHandle {
        if Self::clamp_radius(&mut init.radius) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        // 坏参检查（A1-(2)(3)）：按 arity 走格——扩展槽是 scratch，字节不判 op。
        // 第一遍：验 op/扩展槽空间/easing id，收集合法边界位图（zero-tail 全为 END = 合法边界）。
        let mut bad = xform.len() > crate::xform::SLOTS_PER_SEG;
        let mut boundaries: u16 = 0;
        let mut k = 0usize;
        while !bad && k < xform.len() {
            let s = &xform[k];
            if !crate::xform::op_implemented(s.op) {
                bad = true;
                break;
            }
            boundaries |= 1 << k;
            let ar = crate::xform::ARITY[s.op as usize] as usize;
            if ar > 0 {
                if k + ar >= crate::xform::SLOTS_PER_SEG {
                    bad = true; // 扩展槽越出段（如 STEP 在槽 15）
                    break;
                }
                if ((s.args[1] >> 16) as u8) >= 8 {
                    bad = true; // easing id 越界
                    break;
                }
            }
            k += 1 + ar;
        }
        // zero-tail（含恰好越出提供长度的走格终点）：全零 = END，合法边界
        for t in xform.len()..crate::xform::SLOTS_PER_SEG {
            boundaries |= 1 << t;
        }
        // 第二遍：LOOP target 必须落在边界上
        if !bad {
            let mut k = 0usize;
            while k < xform.len() {
                let s = &xform[k];
                if s.op == crate::xform::OP_LOOP {
                    let t = s.args[0];
                    if !(0..crate::xform::SLOTS_PER_SEG as i32).contains(&t)
                        || boundaries & (1 << t) == 0
                    {
                        bad = true;
                        break;
                    }
                }
                k += 1 + crate::xform::ARITY[s.op as usize] as usize;
            }
        }
        if bad {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return BulletHandle::NULL;
        }
        let Some(seg) = self.xforms.alloc() else {
            self.diag.pool_full[POOL_XFORM] = self.diag.pool_full[POOL_XFORM].wrapping_add(1);
            self.last_status = STATUS_POOL_FULL;
            return BulletHandle::NULL;
        };
        let dst = self.xforms.seg_slots_mut(seg);
        dst[..xform.len()].copy_from_slice(xform);
        dst[xform.len()..].fill(Default::default()); // 尾零 = 天然 END（复用段写满义务）
        init.transform_head = seg;
        init.xform_wait = 0;
        init.xform_next = 0;
        match self.bullets.alloc(init) {
            Some(h) => h,
            None => {
                self.xforms.free(seg); // 先段后弹的回滚半边
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

    /// 脉冲一条信号通道（相位 4 前有效——导演槽/ECL；边沿语义见 `signals` 字段文档）。
    /// P4-b：坏通道 no-op + 计数。debug 断言相位窗口：相位 4 之后的脉冲当帧蒸发，
    /// 正路是上层读事件、次帧经导演/ECL 转发。
    pub fn pulse_signal(&mut self, ch: usize) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.phase_guard <= PH_XFORM,
            "pulse_signal 晚于相位 4：本帧无人能听见（请次帧经导演/ECL 转发）"
        );
        if ch >= SIGNAL_CHANNELS {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        self.signals[ch] = self.frame.wrapping_add(1);
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

    // ── 相位函数（pub(crate)，每个先 phase_enter 保序）────────────────────
    pub(crate) fn begin(&mut self) {
        self.phase_enter(PH_BEGIN);
        self.hits_len = 0;
        self.events_len = 0;
    }
    pub(crate) fn advance(&mut self) {
        self.phase_enter(PH_ADVANCE);
        self.frame += 1;
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::math::Fx;

    /// 造一颗停在 (x,y) 的哑弹（半径 2）。
    pub(crate) fn bullet_at(
        w: &mut crate::step::World,
        x: i32,
        y: i32,
    ) -> crate::bullets::BulletHandle {
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
        })
    }

    pub(crate) fn spawn_enemy(
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

    pub(crate) fn spawn_field(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::test_support::*;

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
}
