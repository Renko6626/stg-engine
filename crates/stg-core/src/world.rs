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
use crate::math::{Angle, Fx};
use crate::player::PlayerState;
use crate::reqs::{REQS_CAP, RenderReq};
use crate::rng::Pcg32;
use crate::shots::{ShotHandle, ShotInit, ShotPool};

mod cleanup;
mod collide;
mod integrate;
mod motion;
mod player;
mod settle;
mod transform;
mod view;

pub use view::WorldView;

// ── 常量：池 id / 错误码 / 场界（D7 中轴原点，384×448 + 越界边距）──────────
pub const POOL_BULLET: usize = 0;
pub const POOL_SHOT: usize = 1;
pub const POOL_ENEMY: usize = 2;
pub const POOL_FIELD: usize = 3;
pub const POOL_XFORM: usize = 4;
pub const POOL_ITEM: usize = 5;
/// ECL 任务池（M1 T2；`pool_full` 数组容量 8，恰余两位——够用无需扩容）。
pub const POOL_TASK: usize = 6;
pub const STATUS_OK: u16 = 0;
pub const STATUS_POOL_FULL: u16 = 1;
pub const STATUS_STALE_HANDLE: u16 = 2;
pub const STATUS_BAD_ARGS: u16 = 3;
/// 缓冲满截断（D12：`emit_req` 满 → 丢弃 + 本状态 + `diag.reqs_dropped`）。
pub const STATUS_TRUNCATED: u16 = 4;

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
///   写 API**——由 `PlayerState::spawn` 从 `WorldTables::CharacterCfg` 赋值（M0-17 迁表），
///   上限由 `WorldTables::validate()` 的角色半径腿 + `spawn_radii_match_tables_v0_bitwise`
///   位等测试钉死（原 player.rs 编译期断言已随常量迁表退役）。**自机写口已收紧**（刀 A，
///   2026-07-21）：`WorldBody.players` 字段为 `pub(crate)`，断层线以上只能经 `set_player_power`
///   写 API（钳 POWER_MAX）改 power、经 `players()` 只读访问器读态——与四池"只能走写 API"同为
///   类型系统强制。`PlayerState` 内部字段仍 `pub`，但数组已封 → 外部无 `&mut` 路径可达，
///   `players()` 交出的 `&PlayerState` 只读不可写。
pub const MAX_ENTITY_RADIUS: Fx = Fx::from_int(1024);

/// 信号黑板通道数（D4 11b）。
pub const SIGNAL_CHANNELS: usize = 8;

/// 全局变量竞技场槽数（D12/A2，M1 ECL 状态地基）。
pub const GLOBALS_CAP: usize = 1024;

/// globals 段纪律（甲案，M1.5）：`[0, GLOBALS_SYS_SEGMENT)` 是**系统段**——只准 game 层
/// 经 `WorldBody::set_var`（世界 API，本模块下方，调用方是 harness/绑定层场景搭建代码）写，
/// **脚本经 `SYS_SET_VAR` 写这段一律 no-op + `diag.contract_viol` 计数**（P4-b：脚本作者
/// 违约→确定性安全结果，不 Fault-kill 任务——见 `ecl::syscall::dispatch` 的 `SYS_SET_VAR` 分支
/// 与 `ecl-ops.md` "作者须知"）。`GLOBALS_SYS_SEGMENT..GLOBALS_CAP` 是**自由段**，脚本读写皆
/// 无限制。`SYS_GET_VAR`（脚本读）不受本纪律约束——两段皆可读，只有脚本写系统段被挡。
///
/// 系统段内还含 RANK（难度）槽 [`GVAR_RANK`]——场景搭建代码经世界 API `set_var(GVAR_RANK, ..)`
/// 写（见 `stg-harness` 彩虹风铃卡 scene 2 建场），脚本只 `get_var(GVAR_RANK)` 读后自决
/// （spec 拍板 5）。定义均迁至 `crate::consts`（C14 单一注册表）。
pub use crate::consts::{GLOBALS_SYS_SEGMENT, GVAR_RANK};

/// 场界半宽（D7 中轴坐标系，单位 px，x ∈ [-192, 192]）。
pub const FIELD_HALF_W: i32 = 192; // x ∈ [-192, 192]
/// 场界高（D7 中轴坐标系，单位 px，y ∈ [0, 448]）。
pub const FIELD_HEIGHT: i32 = 448; // y ∈ [0, 448]
/// 飞行物越界回收边距（单位 px）。表现层通常不需要，py 观测器可用来解释实体消失。
pub const OOB_MARGIN: i32 = 64; // 越界回收边距
/// 敌人专用越界边距（回收兜底，单位 px）。系统性宽于飞行物的 64px：入场/绕场编排要在场外
/// 起舞，回收主导靠纪律——敌人主协程返回即自燃（ZUN ECL 语义，D9 落地：
/// `ecl::vm::run_tasks` 的 `Exec::End` 分支，命中 `enemies.main_task` 即标 `ENEMY_DYING`）；
/// 本常量只是防脚本失手泄漏的安全网。表现层通常不需要，py 观测器可用来解释实体消失。
pub const ENEMY_OOB_MARGIN: i32 = 256;
pub(crate) const POC_LINE_Y: i32 = 128; // 回收线（PoC）：ALIVE 自机 y 低于此线 → 全场道具磁吸

// ── 相位索引（A4 v2，0-based；PhaseGuard 押运）───────────────────────────
/// 相位数。**debug 专用**：唯一使用者是 `phase_enter` 里那个 `#[cfg(debug_assertions)]` 块
/// （P2：debug 押运 §3.5 时序，release 不检查），故条件编译跟着走——否则 release 下它真的
/// 没有任何使用者，`cargo clippy --release` 报一条 `dead_code`（follow-ups **F7**/**C3**）。
/// 取 `#[cfg]` 而非 `#[allow(dead_code)]`：后者把"release 里没人用"这个事实盖住了。
#[cfg(debug_assertions)]
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
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct DiagCounters {
    pub pool_full: [u32; 8], // 按池 id
    pub contract_viol: u32,
    pub hits_overflow: u32,   // hits 满丢弃计数（P4-a）
    pub events_overflow: u32, // events 满丢弃计数（P4-a）
    /// ECL 任务确定性报错被杀的累计计数（M1 T2；`derive(Checksum)` 自动入校验和，
    /// 与 owner 死亡的静默回收物理区分——owner 死不计这里）。
    pub task_faults: u32,
    /// reqs 满丢弃计数（P4-a/D12 名 `reqs_dropped`——表现可以掉，确定性不能破，
    /// 两机必须丢得一样多，故**必须入校验和**、不得 skip）。
    pub reqs_dropped: u32,
}

/// 世界本体（最小切片）。构造走 `step::World::new`（堆零初始化 + 播种 rng）。
#[repr(C)]
#[derive(crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct WorldBody {
    /// 写口唯相位/API;读走 `frame()`。
    pub(crate) frame: u32,
    /// I3:状态随快照;外部不可触(唯一转发出口见 `rand_range`,不交出 `&Pcg32` 本身)。
    pub(crate) rng: Pcg32,
    /// 全局变量竞技场（A2）——纯 i32 槽，语义归脚本，世界自身不读不写；脚本写读走
    /// `set_var`/`get_var`。零初始化合法。断层线以上只读走 `view().globals()`（D6，
    /// 2026-07-25：收 `pub(crate)`，绕开 `get_var` 的 `&mut self`/计数副作用）。
    pub(crate) globals: [i32; GLOBALS_CAP],
    pub(crate) bullets: BulletPool,
    pub(crate) players: [PlayerState; crate::MAX_PLAYERS],
    /// boss 公告板（A2）——脚本写（`boss_set`）、UI 读、世界自身不读。零初始化合法。
    /// 断层线以上只读走 `view().boss_ui()`（D6，2026-07-25：收 `pub(crate)`）。
    pub(crate) boss_ui: [crate::boss::BossUiSlot; crate::boss::MAX_BOSSES],
    pub(crate) shots: ShotPool,
    pub(crate) enemies: EnemyPool,
    pub(crate) fields: FieldPool,
    /// 道具池（D7）。与四实体池同级 `pub(crate)`——表现层经 `view()` 只读访问器读。
    pub(crate) items: crate::items::ItemPool,
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
    pub(crate) frame_events: [Event; EVENTS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 events 一并 skip（A5）")]
    pub(crate) frame_events_len: u16,
    /// 符卡计器槽（每 boss 一个；spec 2026-07-24）。生而封口，读经 `view().spells()`。
    pub(crate) spells: [crate::spell::SpellSlot; crate::boss::MAX_BOSSES],
    /// 每槽持久单调代际计数器（ABA 修复，复审 Task 2）：`spell_begin_internal` 成功时
    /// `wrapping_add(1)` 后把新值戳进 `spells[slot].epoch`。**只增不清**——不随槽结算归零，
    /// 与 `spells[slot]` 本身（inactive 时全字段清零）刻意分层：槽内 `epoch` 是"当前占用者的
    /// 世代"，这里是"这个槽历史上一共发过多少代"。全零初始化合法（首次 begin 即从 1 起算）。
    pub(crate) spell_seq: [u16; crate::boss::MAX_BOSSES],
    /// 表现锚点（整局流程刀 spec §4）：存读档/回滚/中段启动后表现层重同步的世界侧真相。
    /// 仅 5xx 族 syscall 写、任何相位不读（判别式测试押运）；P6 全量入校验和，facing 先例。
    pub(crate) bgm_id: u16,
    pub(crate) bg_id: u16,
    pub(crate) bg_phase: u16,
    pub(crate) bg_phase_frame: u32,
    /// 时停剩余帧（自机能力刀，2026-09-03）：`[0]` = 玩家技能（冻 B+C）、
    /// `[1]` = ECL 演出（冻 A+B）。**冻结掩码是推导的、不存**——见
    /// [`Self::actor_frozen`]/[`Self::scene_frozen`]/[`Self::shots_frozen`]；
    /// 存一份掩码只会多一个与倒计时不同步的机会。
    ///
    /// **两个都在相位 0 `begin` 无条件递减**，不属于任何冻结组：若各自跟组走，
    /// 两边同时开启时 A 被演出冻住 ⇒ 技能倒计时不走、C 被技能冻住 ⇒ 演出倒计时
    /// 不走，**世界永远解不开**（spec §5）。
    pub(crate) freeze_left: [u16; 2],
    #[checksum(skip = "纯输出缓冲，回滚重演确定性再生（P6/§6.2 通道 B）")]
    pub(crate) reqs: [RenderReq; REQS_CAP],
    #[checksum(skip = "纯输出缓冲，len 随 reqs 一并 skip（通道 B）")]
    pub(crate) reqs_len: u16,
    /// 诊断计数器。断层线以上只读走 `view().diag()`（D6，2026-07-25：收 `pub(crate)`，
    /// `Copy` 按值出）。
    pub(crate) diag: DiagCounters,
    /// 上一次写 API 调用的状态码（P4-b）。断层线以上只读走 `view().last_status()`（D6，
    /// 2026-07-25：收 `pub(crate)`）。
    pub(crate) last_status: u16,
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

    /// xform 序列合法性（两遍 arity 走格）：长度 ≤16、逐 op 已实现（扩展槽 scratch 不判）、
    /// STEP 扩展槽空间、easing id < 8、LOOP target 落边界（zero-tail = 合法 END 边界）。
    /// 纯谓词——计数/status 处置留调用方（create_bullet_with_xform 与 create_bullets_batch 共用）。
    fn xform_args_valid(xform: &[crate::xform::XformSlot]) -> bool {
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
        !bad
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
        if !Self::xform_args_valid(xform) {
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

    /// N×K 网格批量发射器（性能语义原语；ECL syscall `create_bullets_batch` 直通）。
    /// 环 = n_speed=1；列 = n_angle=1；多重环 = 双轴。迭代序 = 角度外层、速度内层 = 池槽
    /// 分配序（I4 契约）。两轴累加器：角度 BAM 回绕、速度 Fx 裸加（溢出 P4-c 域）。
    /// P4：轴零/超池 cap/坏 xform → BAD_ARGS 整体拒（实发 0 零副作用）；额度内池/段满 →
    /// 尽力而为 + 满额短路（剩余批量计数，与逐颗试严格等价）。模板 radius 钳一次。
    /// **段消耗账**：xform 非空时每颗自有段拷贝——一次吃 n_angle×n_speed 个段（段池 2048）。
    /// 无 RNG（路线甲：生成器纯确定）。返回实发数。
    #[allow(clippy::too_many_arguments)] // 批量原语的天然参数面；ECL 绑定层按位打包
    pub fn create_bullets_batch(
        &mut self,
        mut init: BulletInit,
        xform: &[crate::xform::XformSlot],
        n_angle: u16,
        angle0: Angle,
        angle_step: i16,
        n_speed: u16,
        speed0: Fx,
        speed_step: Fx,
    ) -> u16 {
        // 验证序（轴→xform→radius 钳）按 spec 直书，故意不同于 create_bullet_with_xform
        // 的 radius→xform 序：同时坏 radius+坏 xform 时这里 contract_viol 只 +1（xform 拒
        // 先短路）。语义已定案入测试，勿"修正"成对齐单发 API。
        let total = n_angle as u32 * n_speed as u32;
        if n_angle == 0 || n_speed == 0 || total > BulletPool::CAP as u32 {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return 0;
        }
        if !xform.is_empty() && !Self::xform_args_valid(xform) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return 0;
        }
        if Self::clamp_radius(&mut init.radius) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        let mut created: u16 = 0;
        let mut cur_angle = angle0;
        'grid: for _ in 0..n_angle {
            let mut cur_speed = speed0;
            for _ in 0..n_speed {
                let (vx, vy) = crate::math::geom::polar_to_vec(cur_speed, cur_angle);
                init.speed = cur_speed;
                init.angle = cur_angle;
                init.vx = vx;
                init.vy = vy;
                // 逐颗：哑/xform 两路（per-bullet P4-a 语义与单发 API 一致；先段后弹）
                let fail_pool: usize = if xform.is_empty() {
                    init.transform_head = crate::xform::XFORM_NONE;
                    if self.bullets.alloc(init).is_some() {
                        usize::MAX
                    } else {
                        POOL_BULLET
                    }
                } else {
                    match self.xforms.alloc() {
                        None => POOL_XFORM,
                        Some(seg) => {
                            let dst = self.xforms.seg_slots_mut(seg);
                            dst[..xform.len()].copy_from_slice(xform);
                            dst[xform.len()..].fill(Default::default());
                            init.transform_head = seg;
                            init.xform_wait = 0;
                            init.xform_next = 0;
                            if self.bullets.alloc(init).is_some() {
                                usize::MAX
                            } else {
                                self.xforms.free(seg); // 先段后弹回滚（无泄漏）
                                POOL_BULLET
                            }
                        }
                    }
                };
                if fail_pool == usize::MAX {
                    created += 1;
                } else {
                    // 满额短路：同相位无回收，后续必然同败——剩余（含本颗）批量计数，
                    // 确定性严格等价于逐颗试（xform 批弹池满路径逐颗也是还段后计 BULLET）。
                    let remaining = total - created as u32;
                    self.diag.pool_full[fail_pool] =
                        self.diag.pool_full[fail_pool].wrapping_add(remaining);
                    self.last_status = STATUS_POOL_FULL;
                    break 'grid;
                }
                cur_speed = cur_speed + speed_step;
            }
            cur_angle = cur_angle.add_delta(angle_step);
        }
        created
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

    /// 敌人限时缓动位移（D5；杂鱼"飘入-停-飘出"的世界侧状态机，将来 ECL syscall 直通）。
    /// 语义：绝对插值、到点即停（精确终点 + 条件化清 vx/vy，判据 `vel_touched`）；
    /// 进行中重下 = 覆盖重启；dur=0 = 瞬移（合法退化）。
    /// **它是一次完整的运动接管**：武装（两条路径都算）会清掉此前的速度意图——
    /// 既清黏滞位 `vel_touched`、也清在飞的速度插值 `vel_active`。要落地后继续飘，
    /// 把速度动词写在 `move_to` **之后**。P4-b：悬垂/easing 越界 → no-op + 计数。
    /// 目标点不钳制场界。
    pub fn move_enemy_to(&mut self, h: EnemyHandle, x: Fx, y: Fx, dur: u16, easing: u8) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        if easing >= 8 {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        if dur == 0 {
            // 瞬移=硬停（清在飞插值）：即便当前正处于上一次 move_to 的插值中途，
            // 也要清 mv_active——否则 integrate 相下一帧仍走插值分支，用陈旧
            // mv_from/to 把这里刚写的新位置覆盖回旧轨迹上（"瞬移=硬停覆盖"契约破裂）。
            self.enemies.x[i] = x;
            self.enemies.y[i] = y;
            self.enemies.vx[i] = Fx::ZERO;
            self.enemies.vy[i] = Fx::ZERO;
            // 双表示同步（终审 Critical 1）：清 vx/vy 之后必须回填作者视图，否则
            // speed/angle 留着陈值说谎——紧跟一次 move_angle（"只转向、速率不动"）
            // 就会拿那个陈速率把刚停住的敌弹射出去。零向量 ⇒ speed=0、angle 冻结。
            self.backfill_enemy_polar(i);
            self.enemies.mv_active[i] = 0;
            // move_to 是一条全新的位置命令——它之前的速度意图是陈的，由它接管。
            // 到点清速的判据（integrate 相）读的就是这一位。
            self.enemies.vel_touched[i] = 0;
            // 连"在飞的速度插值"一并作废（终审 Important 4 的控制器裁定）：理由与上一行
            // 逐字相同——既然连"表达过速度意图"这个事实都作废，"意图正在执行中"更该作废。
            // 不清的话，本帧刚硬停的敌次帧就被仍在飞的插值器按旧 from/to 写回速度。
            self.enemies.vel_active[i] = 0;
            return;
        }
        self.enemies.mv_from_x[i] = self.enemies.x[i];
        self.enemies.mv_from_y[i] = self.enemies.y[i];
        self.enemies.mv_to_x[i] = x;
        self.enemies.mv_to_y[i] = y;
        self.enemies.mv_t[i] = 0;
        self.enemies.mv_dur[i] = dur;
        self.enemies.mv_easing[i] = easing;
        self.enemies.mv_active[i] = 1;
        // move_to 是一条全新的位置命令——它之前的速度意图是陈的，由它接管。
        // 到点清速的判据（integrate 相）读的就是这一位。
        self.enemies.vel_touched[i] = 0;
        // 同 dur==0 路径：在飞的速度插值一并作废（终审 Important 4）。move_to 是一次
        // **完整**的运动接管。想让敌落地后继续飘，把速度动词写在 move_to **之后**
        // （spec §3.2 样例本来就是那个顺序，故不受影响）。
        self.enemies.vel_active[i] = 0;
    }

    /// 清空敌人的待掉落计数（敌人死亡效果刀）。P4-b：悬垂 → no-op + 计数。
    pub fn clear_enemy_drops(&mut self, h: EnemyHandle) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.enemies.drop_count[i] = [0; crate::items::ITEM_TYPE_COUNT];
    }

    /// 给敌人的待掉落计数**增量**加 `n` 颗 `item_type`（敌人死亡效果刀）。
    ///
    /// P4-b 三处：悬垂 → no-op + 计数；`item_type` 越界 → no-op + 计数；
    /// `n` 先钳进 `[0, u8::MAX]` 再对计数 `saturating_add`。**两步都要**——只钳不饱和会在
    /// 计数接近 255 时溢出（debug 下 panic），只饱和不钳则负数 `as u8` 会回绕成大正数。
    /// 本刀不做"减掉落"，故负 `n` 视同 0（要清空用 [`WorldBody::clear_enemy_drops`]）。
    pub fn add_enemy_drop(&mut self, h: EnemyHandle, item_type: i32, n: i32) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        if item_type < 0 || item_type as usize >= crate::items::ITEM_TYPE_COUNT {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        let add = n.clamp(0, u8::MAX as i32) as u8;
        let slot = &mut self.enemies.drop_count[i][item_type as usize];
        *slot = slot.saturating_add(add);
    }

    /// 脚本显式撒掉落（敌人死亡效果刀）。**不清零、不加分、不发事件**（人类裁定 D-3）。
    /// **不设 dying 门禁**——对已 dying 的敌照撒不误（与 `kill_enemy` 的幂等门禁不同）。
    /// P4-b：悬垂 → no-op + 计数。
    pub fn spill_enemy_drops(&mut self, h: EnemyHandle, tables: &crate::tables::WorldTables) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.spill_drops(i, tables);
    }

    /// 脚本显式触发敌人的完整死亡效果（敌人死亡效果刀）。幂等（已 dying → no-op）。
    /// P4-b：悬垂 → no-op + 计数。
    pub fn kill_enemy_by_handle(&mut self, h: EnemyHandle, tables: &crate::tables::WorldTables) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.kill_enemy(i, tables);
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

    /// 掉落一颗道具（内部核；散布消耗世界 RNG——消耗序 = 调用序 = 结算序，A6）。
    /// P4-a：池满 → NULL + 计数。类型合法性由调用方保证（settle 走表、公开壳已验）。
    pub(crate) fn spawn_drop(
        &mut self,
        x: Fx,
        y: Fx,
        item_type: u8,
        tables: &crate::tables::WorldTables,
    ) -> crate::items::ItemHandle {
        let cfg = &tables.item_cfg[item_type as usize];
        let vx = Fx::from_raw(self.rng.rand_range(131_073) as i32 - 65_536); // ±1.0
        let vy = Fx::ZERO - cfg.eject_speed + Fx::from_raw(self.rng.rand_range(32_769) as i32);
        match self.items.alloc(crate::items::ItemInit {
            x,
            y,
            vx,
            vy,
            item_type,
            magnet_to: crate::items::MAGNET_NONE,
            timer: 0,
        }) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_ITEM] = self.diag.pool_full[POOL_ITEM].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                crate::items::ItemHandle::NULL
            }
        }
    }

    /// 消弹转星星的生成核（D9 趟一，M0-15）：原弹位、零初速、出生即磁吸（调用方给
    /// `magnet_to`——存活自机或 `MAGNET_NONE`）；**无散布无 RNG**（与 `spawn_drop` 的
    /// 关键区别——星星不喷射，直接飞向自机或原地下落）。
    /// P4-a：池满 → 该颗不生成 + 逐颗计数（消弹循环有界，不短路；判别腿
    /// `star_pool_full_counts_every_missing_star`）。
    ///
    /// **这条降级路径是真会被走到的，不是理论边界**（F12，2026-09-03）：转换是 1:1，而
    /// 道具池 cap 1024 比弹池 cap 8192 窄 8 倍 —— demo 局收卡那一帧场上 626 颗弹全转星星，
    /// 道具池还是 512 的时候**四个难度档全部溢出**（Easy 3 / Normal 37 / Hard 67 /
    /// Lunatic 104 颗没生成）。把 cap 抬到 1024 盖住了当前内容的弹数峰值（rank 3 为 814），
    /// 但 **1024 不是结构性保证，只是把线挪远**：任何一次消弹多于道具池余量都会再走到这里。
    /// **这是已知设计边界，不是待修的债**（作者侧口径见 `docs/ecl-lang/6-spell-and-stage.md`
    /// 的 `clear_bullets()` 一节与 `docs/ecl-ops.md` 的 540 号）。
    pub(crate) fn spawn_star_at(&mut self, x: Fx, y: Fx, magnet_to: u8) {
        if self
            .items
            .alloc(crate::items::ItemInit {
                x,
                y,
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                item_type: crate::items::ITEM_STAR,
                magnet_to,
                timer: 0,
            })
            .is_none()
        {
            self.diag.pool_full[POOL_ITEM] = self.diag.pool_full[POOL_ITEM].wrapping_add(1);
            self.last_status = STATUS_POOL_FULL;
        }
    }

    /// 掉落一颗道具（公开写 API；将来 ECL syscall `drop_item` 直通）。
    /// P4-b：坏类型 → NULL + BAD_ARGS（散布 RNG **不**消耗——失败零副作用）。
    pub fn drop_item(
        &mut self,
        x: Fx,
        y: Fx,
        item_type: u8,
        tables: &crate::tables::WorldTables,
    ) -> crate::items::ItemHandle {
        if item_type as usize >= crate::items::ITEM_TYPE_COUNT {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return crate::items::ItemHandle::NULL;
        }
        self.spawn_drop(x, y, item_type, tables)
    }

    /// 全场磁吸（bomb / 导演 / 将来 ECL syscall 的通用入口）：全部未锁定道具锁定该自机。
    /// P4-b：坏索引或目标非 ALIVE → no-op + 计数 + BAD_ARGS。
    pub fn attract_all_items(&mut self, player: usize) {
        if player >= crate::MAX_PLAYERS
            || self.players[player].life_state != crate::player::LIFE_ALIVE
        {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        let nw = self.items.alive.len();
        for w in 0..nw {
            let mut bits = self.items.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.items.magnet_to[i] == crate::items::MAGNET_NONE {
                    self.items.magnet_to[i] = player as u8;
                }
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

    /// ECL 全局变量槽写（D12）。slot ≥ 1024 → no-op + 计数（P4-b）。
    pub fn set_var(&mut self, slot: u16, val: i32) {
        if let Some(g) = self.globals.get_mut(slot as usize) {
            *g = val;
        } else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
        }
    }

    /// 拔火力档（导演/游戏层唯一的自机 power 外部写入口，收 D1）。P4-b：越界 player 索引
    /// → no-op + contract_viol 计数 + last_status=BAD_ARGS（同 set_var/pulse_signal 口径）；
    /// power 上钳 POWER_MAX（`power_tier` index OOB 的根，见本文件顶 MAX_ENTITY_RADIUS 注释）。
    pub fn set_player_power(&mut self, player: usize, power: u16) {
        if player >= crate::MAX_PLAYERS {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        self.players[player].power = power.min(crate::items::POWER_MAX);
    }

    /// 自机只读切片（表现层读自机态的入口；通道 A 最小种子，非完整 WorldView）。
    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    /// 通道 A 只读视图入口（A9）——step 后/相位间经它读五池 SoA；活着期间借 &self 挡住 step。
    pub fn view(&self) -> WorldView<'_> {
        WorldView { body: self }
    }

    /// ECL 全局变量槽读（D12）。slot ≥ 1024 → 0 + 计数——取 &mut self 正是为了
    /// 坏槽计数入校验和（两机必须一样错）。
    pub fn get_var(&mut self, slot: u16) -> i32 {
        match self.globals.get(slot as usize) {
            Some(&v) => v,
            None => {
                self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                self.last_status = STATUS_BAD_ARGS;
                0
            }
        }
    }

    /// boss 公告板整槽写（D12）。slot ≥ MAX_BOSSES → no-op + 计数（P4-b）。
    /// 整槽写入与"复用槽写满"纪律同构——脚本层想改单字段自行先读后写。
    /// 不校验 `ui.enemy` 句柄有效性：世界不读公告板，悬垂由读方按"视同已失效"处置。
    pub fn boss_set(&mut self, slot: u8, ui: crate::boss::BossUiSlot) {
        if let Some(b) = self.boss_ui.get_mut(slot as usize) {
            *b = ui;
        } else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
        }
    }

    /// 找到绑定某敌（索引 + 代）的 active 符卡槽（逃生舱口/读时器/伤害下钳共用，spec
    /// 2026-07-24 §3.1/§4）。按槽升序（I4）；无绑定 None。
    fn spell_slot_bound_to(&self, index: u16, generation: u16) -> Option<usize> {
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
        true
    }

    /// 符卡结算原子包（HP 路径/超时路径/逃生舱口共用，spec §4 结束矩阵）：付分（`captured`
    /// 时 `players[0].score += bonus_now`）+ `push_event`（CAPTURED 或 FAILED+`reason`）+
    /// `emit_req(REQ_SPELL_RESULT)` + 除非 `SPELL_NO_CLEAR` 铺一个全屏消弹 field（`bomb`/
    /// 敌死同租户，复用 `create_field`）+ 槽全字段清零（复用槽写满纪律的另一半：清空亦是
    /// 全字段覆写）。`reason` 仅在 `captured==false` 时写入事件/req（1=资格失 2=超时）。
    pub(crate) fn settle_one_spell(&mut self, slot: usize, captured: bool, reason: i32) {
        let s = self.spells[slot];
        let boss = EnemyHandle {
            index: s.boss_index,
            generation: s.boss_gen,
        };
        let (x, y) = match self.enemies.get(boss) {
            Some(i) => (self.enemies.x[i], self.enemies.y[i]),
            None => (Fx::ZERO, Fx::ZERO),
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
        if s.flags & crate::spell::SPELL_NO_CLEAR == 0 {
            self.create_field(crate::field::fullscreen_clear_field());
        }
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
        let captured = self.spells[slot].capture_ok != 0;
        let reason = if captured {
            0
        } else {
            crate::spell::SPELL_FAIL_CAPTURE_LOST
        };
        self.settle_one_spell(slot, captured, reason);
    }

    /// 读族（`SYS_SPELL_TIMER` 世界侧核）：owner 绑定的 active 槽返回 `frames_left`；
    /// 无绑定 → `-1`（`wait_spell()` 语法糖的判据）。
    pub fn spell_frames_left_of(&self, boss: EnemyHandle) -> i32 {
        match self.spell_slot_bound_to(boss.index, boss.generation) {
            Some(slot) => self.spells[slot].frames_left as i32,
            None => -1,
        }
    }

    /// 最近敌查询（D7 预定的世界查询助手；homing / ECL 瞄敌共用）。
    /// 契约：纯查询零副作用；候选 = 存活且非 dying；平方距离（i64）；并列取低索引（I4）；
    /// 空集 None 不计数（合法世界状态非违约）；返回带 generation 句柄（P1）。
    pub fn nearest_enemy(&self, x: Fx, y: Fx) -> Option<EnemyHandle> {
        let mut best: Option<(usize, i64)> = None;
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.enemies.flags[i] & crate::enemy::ENEMY_DYING != 0 {
                    continue;
                }
                let d2 = crate::math::geom::len_sq(self.enemies.x[i] - x, self.enemies.y[i] - y);
                if best.is_none_or(|(_, bd)| d2 < bd) {
                    best = Some((i, d2));
                }
            }
        }
        best.map(|(i, _)| EnemyHandle {
            index: i as u16,
            generation: self.enemies.generation[i],
        })
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
        if (self.frame_events_len as usize) < EVENTS_CAP {
            self.frame_events[self.frame_events_len as usize] = ev;
            self.frame_events_len += 1;
        } else {
            self.diag.events_overflow = self.diag.events_overflow.wrapping_add(1);
        }
    }

    /// 世界 RNG 唯一外部触点（本刀迁移面探测漏收：`rng` 封 `pub(crate)` 前，
    /// `stg-harness` 场景搭建代码经 `WorldBody.rng` 直取随机弹幕扩散角——发现于 Task 2
    /// 实现期编译红，非 brief 原定产物，机械补齐见 task-2-report.md）。转发
    /// `Pcg32::rand_range`，只交出一次性抽签结果，不交出 `&Pcg32` 本身——I3"状态随快照，
    /// 外部不可触"仍成立（调用方摸不到 rng 内部状态，只能借这一个确定性出口消耗它）。
    ///
    /// **调用即推进世界正典 RNG 流（写操作，I3）**：只应在确定性帧序内调用——同输入回放必须
    /// 同频次、同顺序，否则跨机/回放静默分叉。合法调用方 = 导演闭包/相位内逻辑；外接层（M2/py）
    /// 拿到 `&mut World` 后在帧外随手调它 = 破坏回放，切勿。表现层抖动请用自己的 RNG（I3 双颗）。
    pub fn rand_range(&mut self, n: u32) -> u32 {
        self.rng.rand_range(n)
    }

    /// 通道 B 推送（§6.2）。id 语义世界不解释（含 0——保留无效值，分发器忽略）；
    /// 满 → 确定性丢弃 + `TRUNCATED` + 计数（P4-a/D12），不 panic。成功不动 `last_status`。
    pub fn emit_req(&mut self, id: u16, args: [i32; 6]) {
        if (self.reqs_len as usize) < REQS_CAP {
            self.reqs[self.reqs_len as usize] = RenderReq {
                id,
                seq: self.reqs_len,
                args,
            };
            self.reqs_len += 1;
        } else {
            self.diag.reqs_dropped = self.diag.reqs_dropped.wrapping_add(1);
            self.last_status = STATUS_TRUNCATED;
        }
    }

    /// 通道 B 出口（蓝图 §256）：本帧请求切片。**幂等非消费**——名字沿契约叫 take，
    /// 帧内多次调用返回同一切片；缓冲下帧 `begin` 清空，headless 无人消费 = 零成本。
    pub fn take_requests(&self) -> &[RenderReq] {
        &self.reqs[..self.reqs_len as usize]
    }

    /// 5xx 族锚点写口（syscall 专用；写字段+发 req 一体，保证"字段可 seek、req 可边沿"双通道）。
    pub(crate) fn set_bgm(&mut self, id: u16) {
        self.bgm_id = id;
        self.emit_req(crate::consts::REQ_BGM, [id as i32, 0, 0, 0, 0, 0]);
    }
    pub(crate) fn set_bg(&mut self, id: u16) {
        self.bg_id = id;
        self.emit_req(crate::consts::REQ_BG, [id as i32, 0, 0, 0, 0, 0]);
    }
    pub(crate) fn set_bg_phase(&mut self, n: u16) {
        self.bg_phase = n;
        self.bg_phase_frame = self.frame;
        self.emit_req(crate::consts::REQ_BG_PHASE, [n as i32, 0, 0, 0, 0, 0]);
    }
    /// 表现锚点只读口（bgm_id/bg_id/bg_phase/bg_phase_frame）——`WorldView` 亦转发同名方法。
    pub fn bgm_id(&self) -> u16 {
        self.bgm_id
    }
    pub fn bg_id(&self) -> u16 {
        self.bg_id
    }
    pub fn bg_phase(&self) -> u16 {
        self.bg_phase
    }
    pub fn bg_phase_frame(&self) -> u32 {
        self.bg_phase_frame
    }

    /// 当前帧号只读口(I6;M2 表现层水位协议消费)。写帧号唯 `advance`(相位 10)。
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// 世界大事记出口(A9 契约名 `frame_events`,代码字段同名——D2 漂移已销，2026-09-03)。
    /// 幂等只读,按 `events_len` 切片——数组本体从不清零,切片界即真相,消费者永不见陈旧尾槽。
    /// 缓冲下帧 `begin` 清 len,与 `take_requests`/`hits` 同生命周期(A5)。
    pub fn frame_events(&self) -> &[Event] {
        &self.frame_events[..self.frame_events_len as usize]
    }

    // 三个读口的生产消费者是各相位函数体内的门禁（Task 3 已接线，见 `world/player.rs`
    // 相位 3、`world/integrate.rs` 相位 5、`transform`/`collide`/`settle`/`cleanup` 的早退
    // 与 `step.rs` 相位 2）——`#[cfg_attr(not(test), allow(dead_code))]` 随之撤掉：本仓
    // 不用 allow 盖住"其实有人用"的事实。
    /// A 组（自机主动行为：移动/发弹/用能力）是否冻结 —— ECL 演出（`freeze_left[1]`）。
    pub(crate) fn actor_frozen(&self) -> bool {
        self.freeze_left[1] > 0
    }
    /// C 组（世界演化与裁决：敌/弹/ECL/道具/作用区/背景/自机被动计时/相位 6·7）
    /// 是否冻结 —— 玩家技能（`freeze_left[0]`）。
    pub(crate) fn scene_frozen(&self) -> bool {
        self.freeze_left[0] > 0
    }
    /// B 组（自机弹的飞行）是否冻结 —— **任一方向的时停都冻它**。这不是巧合：
    /// 弹一旦离开枪口就不再属于自机（spec §3）。
    pub(crate) fn shots_frozen(&self) -> bool {
        self.freeze_left[0] > 0 || self.freeze_left[1] > 0
    }

    // ── 相位函数（pub(crate)，每个先 phase_enter 保序）────────────────────
    pub(crate) fn begin(&mut self) {
        self.phase_enter(PH_BEGIN);
        self.hits_len = 0;
        self.frame_events_len = 0;
        self.reqs_len = 0;
        // 时停倒计时挂**真实帧**、不属于任何冻结组（spec §5 的死锁解）。
        self.freeze_left[0] = self.freeze_left[0].saturating_sub(1);
        self.freeze_left[1] = self.freeze_left[1].saturating_sub(1);
    }
    pub(crate) fn advance(&mut self) {
        self.phase_enter(PH_ADVANCE);
        // 背景停滞**不是"什么都不做"就有的**：背景动画由 `frame − bg_phase_frame` 驱动，
        // 而 `frame` 恒增（时停不是"帧不走"，是"世界不演化"）⇒ 只冻别的会让背景照样走。
        // 冻 C 时把锚点同步推进，让"背景经过的时间"这个差值不增长（spec §5）。
        // wrapping：锚点与 frame 同为 u32 且只做差值比较，回绕语义一致。
        if self.scene_frozen() {
            self.bg_phase_frame = self.bg_phase_frame.wrapping_add(1);
        }
        self.frame += 1;
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::math::Fx;

    /// `step` 糖（M0-17 T2）：测试专用，固定喂 `&TABLES_V0`——生产/harness 调用方仍需显式
    /// 传入自己的 `&WorldTables`（D12 既定签名形态，见 `crate::step::step`）。
    pub(crate) fn step_t(w: &mut crate::step::World, input: &crate::input::InputFrame) {
        crate::step::step(
            w,
            &crate::tables::TABLES_V0,
            &crate::ecl::image::EclImage::empty(),
            input,
        )
    }

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
            speed: Fx::ZERO,
            angle: crate::math::Angle::ZERO,
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
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
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

    /// 最小合法 `BulletInit`（半径/生命随便给个能过 P4-b 钳制的值，只为灌池）。
    fn min_bullet_init() -> crate::bullets::BulletInit {
        crate::bullets::BulletInit {
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
            radius: Fx::from_int(2),
            delay: 0,
            life: 0xFFFF,
            flags: 0,
            grazed_by: 0,
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
        }
    }

    /// 最小合法 `ShotInit`。
    fn min_shot_init() -> crate::shots::ShotInit {
        crate::shots::ShotInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 1,
            radius: Fx::from_int(2),
            sprite: 0,
            owner: 0,
            flags: 0,
        }
    }

    /// 最小合法 `EnemyInit`。
    fn min_enemy_init() -> crate::enemy::EnemyInit {
        crate::enemy::EnemyInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
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
            main_task: 0,
            death_script: 0,
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
            score: 0,
        }
    }

    /// 最小合法 `FieldInit`。
    fn min_field_init() -> crate::field::FieldInit {
        crate::field::FieldInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            radius: Fx::from_int(10),
            dmg_per_frame: 0,
            life: 1,
            owner: 0,
            flags: 0,
        }
    }

    /// P4-a（宪法级）：弹池满 → 确定性降级——返回 NULL + 计数 + last_status，不 panic。
    /// 金向量永远走不到这里（实测稳态 ~375/8192），故只能靠本测试守。
    #[test]
    fn create_bullet_pool_full_degrades_deterministically() {
        let mut w = crate::step::World::new(1);
        for _ in 0..crate::bullets::BulletPool::CAP {
            let h = w.body.create_bullet(min_bullet_init());
            assert_ne!(h, crate::bullets::BulletHandle::NULL, "灌池阶段不该失败");
        }
        let before = w.body.diag.pool_full[POOL_BULLET];
        let h = w.body.create_bullet(min_bullet_init());
        assert_eq!(h, crate::bullets::BulletHandle::NULL, "池满须返 NULL");
        assert_eq!(w.body.diag.pool_full[POOL_BULLET], before + 1, "池满须计数");
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
    }

    /// P4-a：自机弹池满 → 确定性降级——返回 NULL + 计数 + last_status，不 panic。
    /// 金向量永远走不到这里，故只能靠本测试守。
    #[test]
    fn create_player_shot_pool_full_degrades_deterministically() {
        let mut w = crate::step::World::new(1);
        for _ in 0..crate::shots::ShotPool::CAP {
            let h = w.body.create_player_shot(min_shot_init());
            assert_ne!(h, crate::shots::ShotHandle::NULL, "灌池阶段不该失败");
        }
        let before = w.body.diag.pool_full[POOL_SHOT];
        let h = w.body.create_player_shot(min_shot_init());
        assert_eq!(h, crate::shots::ShotHandle::NULL, "池满须返 NULL");
        assert_eq!(w.body.diag.pool_full[POOL_SHOT], before + 1, "池满须计数");
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
    }

    /// P4-a：敌人池满（cap 256，实测稳态 3/256）→ 确定性降级——返回 NULL + 计数 +
    /// last_status，不 panic。
    #[test]
    fn create_enemy_pool_full_degrades_deterministically() {
        let mut w = crate::step::World::new(1);
        for _ in 0..crate::enemy::EnemyPool::CAP {
            let h = w.body.create_enemy(min_enemy_init());
            assert_ne!(h, crate::enemy::EnemyHandle::NULL, "灌池阶段不该失败");
        }
        let before = w.body.diag.pool_full[POOL_ENEMY];
        let h = w.body.create_enemy(min_enemy_init());
        assert_eq!(h, crate::enemy::EnemyHandle::NULL, "池满须返 NULL");
        assert_eq!(w.body.diag.pool_full[POOL_ENEMY], before + 1, "池满须计数");
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
    }

    /// P4-a：作用区池满（cap 16，实测稳态 1/16）→ 确定性降级——返回 NULL + 计数 +
    /// last_status，不 panic。
    #[test]
    fn create_field_pool_full_degrades_deterministically() {
        let mut w = crate::step::World::new(1);
        for _ in 0..crate::field::FieldPool::CAP {
            let h = w.body.create_field(min_field_init());
            assert_ne!(h, crate::field::FieldHandle::NULL, "灌池阶段不该失败");
        }
        let before = w.body.diag.pool_full[POOL_FIELD];
        let h = w.body.create_field(min_field_init());
        assert_eq!(h, crate::field::FieldHandle::NULL, "池满须返 NULL");
        assert_eq!(w.body.diag.pool_full[POOL_FIELD], before + 1, "池满须计数");
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
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
        assert_eq!(w.body.frame_events_len, 0);
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
        assert_eq!(w.body.frame_events_len, 1);
        assert_eq!(w.body.frame_events[0].kind, EVT_PLAYER_DIED);
    }

    /// P4-a：`push_event` 溢出 → 停收 + 计数，不 panic（`push_hit` 的同构缺口，B2）。
    #[test]
    fn events_push_overflow_counts_and_drops() {
        use crate::events::{EVENTS_CAP, Event};
        let mut w = crate::step::World::new(1);
        w.body.frame_events_len = EVENTS_CAP as u16;
        w.body.push_event(Event {
            kind: crate::events::EVT_ENEMY_DIED,
            a_index: 0,
            a_gen: 0,
            x: Fx::ZERO,
            y: Fx::ZERO,
            data: [0, 0],
        });
        assert_eq!(w.body.frame_events_len, EVENTS_CAP as u16, "满后未增");
        assert_eq!(w.body.diag.events_overflow, 1, "须计一次溢出");
    }

    /// set_player_power 钳边界：恰 POWER_MAX 原样写入；超一格被钳（判别 min 是否真在——
    /// 换成裸写 `self.players[player].power = power` 即红）。
    #[test]
    fn set_player_power_clamps_to_power_max() {
        let mut w = crate::step::World::new(1);
        w.body.set_player_power(0, crate::items::POWER_MAX);
        assert_eq!(
            w.body.players[0].power,
            crate::items::POWER_MAX,
            "恰满档原样写入"
        );
        w.body.set_player_power(0, crate::items::POWER_MAX + 1);
        assert_eq!(
            w.body.players[0].power,
            crate::items::POWER_MAX,
            "超 POWER_MAX 被钳（防 power_tier index OOB）"
        );
    }

    /// set_player_power 越界 player 索引 → P4-b 确定性安全结果：no-op + contract_viol +1 +
    /// last_status=BAD_ARGS（同 set_var/pulse_signal 守卫口径）。
    #[test]
    fn set_player_power_oob_player_is_guarded_no_op() {
        let mut w = crate::step::World::new(1);
        let last = crate::MAX_PLAYERS - 1;
        let before = w.body.players[last].power;
        let cv0 = w.body.diag.contract_viol;
        w.body.set_player_power(crate::MAX_PLAYERS, 200); // 恰过界
        assert_eq!(w.body.players[last].power, before, "越界不写任何真槽");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, STATUS_BAD_ARGS);
    }

    /// players() 只读访问器：返回 MAX_PLAYERS 长切片，内容与内部一致。
    #[test]
    fn players_accessor_returns_full_slice() {
        let mut w = crate::step::World::new(1);
        w.body.set_player_power(0, 123);
        let ps = w.body.players();
        assert_eq!(ps.len(), crate::MAX_PLAYERS);
        assert_eq!(ps[0].power, 123);
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
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
            score: 100,
        };
        let h = w.body.create_enemy(init);
        assert_ne!(h, crate::enemy::EnemyHandle::NULL);
        step_t(&mut w, &crate::input::InputFrame::empty(0));
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
        step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.body.fields.get(h), None); // 活一帧后 cleanup 回收
    }

    #[test]
    fn field_life_n_survives_n_frames() {
        let mut w = crate::step::World::new(1);
        let h = spawn_field(&mut w, 0, 100, 20, crate::field::FIELD_CLEAR_BULLETS, 3);
        for _ in 0..2 {
            step_t(&mut w, &crate::input::InputFrame::empty(0));
            assert!(w.body.fields.get(h).is_some()); // 前 2 帧仍在
        }
        step_t(&mut w, &crate::input::InputFrame::empty(0));
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
            hurtbox: Fx::from_int(30000), // 受击半径越界，体碰半径不越界
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
            score: 0,
        });
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.hurtbox[i], MAX_ENTITY_RADIUS); // P4-b 钳制
        assert_eq!(w.body.diag.contract_viol, 1);
    }

    /// 四口径：三敌取最近（非圆心重合）/ 等距取低索引 / dying 跳过 / 空场 None。
    #[test]
    fn nearest_enemy_contract() {
        let mut w = crate::step::World::new(1);
        assert!(
            w.body.nearest_enemy(Fx::ZERO, Fx::ZERO).is_none(),
            "空场 None"
        );
        let _a = spawn_enemy(&mut w, 0, 100, 5); // 距 (0,0) = 100
        let b = spawn_enemy(&mut w, 0, 60, 5); // 距 60 ← 最近
        let _c = spawn_enemy(&mut w, 80, 0, 5); // 距 80
        assert_eq!(w.body.nearest_enemy(Fx::ZERO, Fx::ZERO), Some(b), "取最近");
        // dying 跳过：把 b 标 dying → 次近 c 当选
        let ib = w.body.enemies.get(b).unwrap();
        w.body.enemies.flags[ib] |= crate::enemy::ENEMY_DYING;
        let c_again = w.body.nearest_enemy(Fx::ZERO, Fx::ZERO).unwrap();
        assert_eq!(
            w.body.enemies.get(c_again).map(|i| w.body.enemies.x[i]),
            Some(Fx::from_int(80))
        );
        // 等距取低索引：清场后摆两个等距敌
        let mut w2 = crate::step::World::new(1);
        let d = spawn_enemy(&mut w2, -50, 0, 5);
        let _e = spawn_enemy(&mut w2, 50, 0, 5);
        assert_eq!(
            w2.body.nearest_enemy(Fx::ZERO, Fx::ZERO),
            Some(d),
            "等距取低索引"
        );
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

    /// WorldView 管线：view() 正确接线各池 + players 委派；World::view 与 WorldBody::view 同源。
    #[test]
    fn view_exposes_pools_and_players() {
        let mut w = crate::step::World::new(1);
        let _b = crate::world::test_support::bullet_at(&mut w, 5, 7);
        let _e = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 3);

        let vb = w.body.view();
        assert_eq!(vb.bullets().iter_alive().count(), 1);
        assert_eq!(vb.enemies().iter_alive().count(), 1);
        // 裸切片指向该弹 x（判别 bullets() 未错接 enemies()）
        let bi = vb.bullets().iter_alive().next().unwrap();
        assert_eq!(vb.bullets().x()[bi], Fx::from_int(5));
        // players() 委派刀 A 访问器
        assert_eq!(vb.players().len(), crate::MAX_PLAYERS);
        // World::view 委派 == WorldBody::view
        assert_eq!(w.view().bullets().iter_alive().count(), 1);
    }

    #[test]
    fn emit_req_records_id_seq_args_in_push_order() {
        let mut w = crate::step::World::new(1);
        w.body.emit_req(7, [1, 2, 3, 4, 5, 6]);
        w.body.emit_req(8, [-1, -2, -3, -4, -5, -6]);
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(
            (reqs[0].id, reqs[0].seq, reqs[0].args),
            (7, 0, [1, 2, 3, 4, 5, 6])
        );
        assert_eq!(
            (reqs[1].id, reqs[1].seq, reqs[1].args),
            (8, 1, [-1, -2, -3, -4, -5, -6])
        );
    }

    #[test]
    fn emit_req_overflow_drops_counts_and_sets_truncated() {
        use crate::reqs::REQS_CAP;
        let mut w = crate::step::World::new(1);
        for i in 0..REQS_CAP {
            w.body.emit_req(1, [i as i32, 0, 0, 0, 0, 0]);
        }
        assert_eq!(w.body.diag.reqs_dropped, 0);
        w.body.emit_req(2, [999, 0, 0, 0, 0, 0]);
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), REQS_CAP, "溢出后 len 停在 cap");
        assert_eq!(w.body.diag.reqs_dropped, 1);
        assert_eq!(w.body.last_status, STATUS_TRUNCATED);
        assert_eq!(
            reqs[REQS_CAP - 1].args[0],
            (REQS_CAP - 1) as i32,
            "已有内容不受扰"
        );
        assert_eq!(reqs[REQS_CAP - 1].seq, (REQS_CAP - 1) as u16);
    }

    #[test]
    fn begin_clears_reqs_and_take_requests_is_idempotent() {
        let mut w = crate::step::World::new(1);
        w.body.emit_req(7, [0; 6]);
        let (p1, l1) = {
            let r = w.body.take_requests();
            (r.as_ptr(), r.len())
        };
        let r2 = w.body.take_requests();
        assert_eq!(
            (p1, l1),
            (r2.as_ptr(), r2.len()),
            "帧内幂等：同一切片（蓝图 §256）"
        );
        w.body.begin();
        assert!(w.body.take_requests().is_empty(), "begin 清空通道 B");
    }

    #[test]
    fn emit_req_is_invisible_to_checksum() {
        let mut w = crate::step::World::new(1);
        let c0 = w.checksum();
        w.body.emit_req(9, [1, 2, 3, 4, 5, 6]);
        assert_eq!(
            w.checksum(),
            c0,
            "reqs/reqs_len 是 checksum-skip 纯输出（P6）"
        );
    }

    #[test]
    fn copy_into_restores_world_with_no_stale_reqs() {
        let mut w = crate::step::World::new(1);
        let mut dst = crate::step::World::new(1);
        w.body.emit_req(7, [0; 6]);
        dst.body.emit_req(3, [9; 6]); // 预污染:恢复目标自带陈旧输出,清零必须由 copy_into 完成
        w.copy_into(&mut dst);
        assert!(
            dst.body.take_requests().is_empty(),
            "恢复出的 World 必须无陈旧通道 B 输出（同 hits/events 契约，见 step.rs copy_into 注释）"
        );
    }

    /// P4-b：死亡效果四写 API 的悬垂句柄腿——各自 no-op + `contract_viol` +
    /// `STATUS_STALE_HANDLE`（照 `move_to_bad_args_contract` 口径）。四条分开跑，
    /// 免得某一个漏写 guard 被另一个的计数掩盖。
    #[test]
    fn enemy_drop_and_kill_apis_degrade_on_stale_handle() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body.enemies.free(h);

        let mut expect = w.body.diag.contract_viol;
        let mut check = |w: &mut crate::step::World, what: &str| {
            expect += 1;
            assert_eq!(w.body.diag.contract_viol, expect, "{what} 应计一次 viol");
            assert_eq!(
                w.body.last_status,
                crate::world::STATUS_STALE_HANDLE,
                "{what} 的 last_status"
            );
        };

        w.body.clear_enemy_drops(h);
        check(&mut w, "clear_enemy_drops");
        w.body.add_enemy_drop(h, crate::items::ITEM_POINT as i32, 1);
        check(&mut w, "add_enemy_drop");
        w.body.spill_enemy_drops(h, &crate::tables::TABLES_V0);
        check(&mut w, "spill_enemy_drops");
        w.body.kill_enemy_by_handle(h, &crate::tables::TABLES_V0);
        check(&mut w, "kill_enemy_by_handle");

        assert_eq!(
            w.body.items.iter_alive().count(),
            0,
            "悬垂句柄不得撒出任何道具"
        );
        assert!(w.body.frame_events().is_empty(), "悬垂句柄不得发死亡事件");
    }

    #[test]
    fn rand_range_forwards_world_rng_stream() {
        let mut w = crate::step::World::new(7);
        let mut reference = crate::rng::Pcg32::new(7, crate::world::RNG_SEQ);
        assert_eq!(w.body.rand_range(384), reference.rand_range(384));
        assert_eq!(
            w.body.rand_range(1000),
            reference.rand_range(1000),
            "同流续抽"
        );
    }

    /// C9：**负 `speed` = 倒飞**（方向翻 180°），三个入口口径一致——本条钉的是接口契约的
    /// 一个无声角落。
    ///
    /// 不是 bug（东方语义里负速有用），但此前**全链无测试**：若哪天有人在 `polar_to_vec`
    /// 或某个入口加一句"负速钳零"，全套测试仍绿、行为已变。
    ///
    /// **判别力**：断言的是**逐位相反数**（`v(-s, θ) == -v(s, θ)`），不是"vx < 0" 这种
    /// 方向性弱断言——钳零那种改法会让 vx 变成 0、两侧都不等；而 `speed.abs()` 那种改法
    /// 会让两侧相等（同号），同样红。批量腿另外押住 `speed_step` 负步**跨零**：五层从
    /// +2.0 每层 −1.0，第三层恰好 0、第四五层为负，覆盖"跨零那一步没被特判掉"。
    #[test]
    fn negative_speed_means_reversed_direction_on_every_entry() {
        use crate::math::geom::polar_to_vec;

        // ① 数学核：负速与正速逐位互为相反数（三个入口最终都落到这里）
        for ang in [Angle::ZERO, Angle(9000), Angle::QUARTER, Angle(50000)] {
            let (px, py) = polar_to_vec(Fx::from_int(3), ang);
            let (nx, ny) = polar_to_vec(Fx::from_int(-3), ang);
            assert_eq!(
                nx.raw(),
                -px.raw(),
                "负速 vx 必须是正速的逐位相反数 @{ang:?}"
            );
            assert_eq!(
                ny.raw(),
                -py.raw(),
                "负速 vy 必须是正速的逐位相反数 @{ang:?}"
            );
        }

        // ② 单发入口：直填负 speed，refresh 后 v 与正速反向
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 0);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.angle[i] = Angle::ZERO; // 朝 +x
        w.body.bullets.speed[i] = Fx::from_int(-3);
        w.body.refresh_vel_from_polar(i);
        assert_eq!(
            w.body.bullets.vx[i],
            Fx::from_int(-3),
            "朝 +x 的负速弹必须往 −x 飞（倒飞），不是停住也不是取绝对值"
        );

        // ③ 批量入口：speed_step 负步跨零（+2/+1/0/−1/−2 五层，同一角度）
        let mut w2 = crate::step::World::new(1);
        let n = w2.body.create_bullets_batch(
            min_bullet_init(),
            &[],
            1,
            Angle::ZERO,
            0,
            5,
            Fx::from_int(2),
            Fx::from_int(-1),
        );
        assert_eq!(n, 5);
        let vxs: Vec<i32> = w2
            .body
            .bullets
            .iter_alive()
            .map(|k| w2.body.bullets.vx[k].raw())
            .collect();
        let one = Fx::from_int(1).raw();
        assert_eq!(
            vxs,
            vec![2 * one, one, 0, -one, -2 * one],
            "负步跨零必须一路穿过去：第三层恰 0、后两层反向，没有任何一层被钳住"
        );
    }

    /// B9：**`contract_viol` 跨类别各计一次**（半径越界 +1、坏 xform +1，同一次调用共 +2）。
    ///
    /// `clamp_radius` 的注释里那句"只计一次"说的是**同一类别内多个半径字段合并算一次**，
    /// 从没覆盖"跨类别是否各自计数"这条轴——现状（各计一次）站得住，但此前无测试锁死：
    /// 下次改动若悄悄把它并成"整次调用最多计 1"，没有任何东西会红，而 `diag` 的计数值
    /// **进校验和**（P6），那是一次静默的世界线变更。
    ///
    /// 第二条腿顺带钉住**两条路径的验证序相反**这件既定事实（`world.rs` 的批量入口注释
    /// 明写"故意不同于单发 API，勿修正成对齐"）：`create_bullets_batch` 是 xform 先拒、
    /// 短路返回，同样一对坏参数只 +1。
    ///
    /// **判别力**：两条断言的期望值不同（2 vs 1）。若把跨类别改成"最多计 1"，第一条红；
    /// 若有人"顺手对齐"两条路径的验证序，第二条红。
    #[test]
    fn contract_viol_counts_once_per_category_within_one_call() {
        // 坏 xform：ARITY>0 的 op 的 easing id 越界（args[1] 高 16 位 >= 8）
        let bad_xform = [crate::xform::XformSlot {
            wait: 0,
            op: crate::xform::OP_STEP_SPEED,
            _pad: 0,
            args: [Fx::from_int(1).raw(), 2 | (8 << 16)],
        }];

        // ① 单发入口（验证序 radius → xform）：两类各计一次 ⇒ +2
        let mut w = crate::step::World::new(1);
        let mut init = min_bullet_init();
        init.radius = Fx::from_int(-1); // 负半径 → 钳 0 + 计一次
        let before = w.body.diag.contract_viol;
        let h = w.body.create_bullet_with_xform(init, &bad_xform);
        assert_eq!(h, crate::bullets::BulletHandle::NULL, "坏 xform 整体拒");
        assert_eq!(
            w.body.diag.contract_viol - before,
            2,
            "半径越界与坏 xform 是两个类别，同一次调用各计一次"
        );

        // ② 批量入口（验证序 xform → radius，xform 拒先短路）：同一对坏参数只 +1
        let mut w2 = crate::step::World::new(1);
        let mut init2 = min_bullet_init();
        init2.radius = Fx::from_int(-1);
        let before2 = w2.body.diag.contract_viol;
        let n = w2.body.create_bullets_batch(
            init2,
            &bad_xform,
            1,
            crate::math::Angle::ZERO,
            0,
            1,
            Fx::from_int(1),
            Fx::ZERO,
        );
        assert_eq!(n, 0, "坏 xform 整体拒，实发 0");
        assert_eq!(
            w2.body.diag.contract_viol - before2,
            1,
            "批量入口的验证序是 xform 先拒、短路返回——半径那次根本没走到，故只 +1。\
             这条不对称是既定语义（见 create_bullets_batch 的实现注释），不是待修的 bug"
        );
    }

    /// 倒计时挂**真实帧**、不属于任何冻结组——两边同时开时若各自跟组走会互相冻死
    /// （spec §5「死锁与它的解」）。本条守的就是"它在相位 0 无条件递减"。
    #[test]
    fn freeze_countdowns_tick_in_begin_unconditionally() {
        let mut w = crate::step::World::new(1);
        w.body.freeze_left = [2, 3];
        // ⚠️ `begin()` 头一句是 `phase_enter(PH_BEGIN)`，它在 debug 下断言
        // `phase_guard == 0` 并推进。连调多次必须每次把护栏拨回相位 0——本模块其余
        // 直调相位函数的测试是同款写法。
        let tick = |w: &mut crate::step::World| {
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = PH_BEGIN;
            }
            w.body.begin();
        };
        tick(&mut w);
        assert_eq!(w.body.freeze_left, [1, 2], "两个倒计时都必须在相位 0 递减");
        tick(&mut w);
        assert_eq!(w.body.freeze_left, [0, 1]);
        tick(&mut w);
        assert_eq!(w.body.freeze_left, [0, 0], "到 0 后饱和，不回绕");
    }

    /// 掩码是**推导**的：A 冻 ⇔ left[1]>0、C 冻 ⇔ left[0]>0、B 冻 ⇔ 任一 >0。
    /// 判别力：四种组合逐个断言——只测"全零"与"全非零"的话，把 A/C 写反照样绿。
    #[test]
    fn freeze_mask_is_derived_from_the_two_countdowns() {
        let mut w = crate::step::World::new(1);
        let cases = [
            ([0u16, 0u16], (false, false, false)),
            ([5, 0], (false, true, true)), // 玩家技能：冻 B+C，A 跑
            ([0, 5], (true, false, true)), // ECL 演出：冻 A+B，C 跑
            ([5, 5], (true, true, true)),  // 全场静止
        ];
        for (left, (a, c, b)) in cases {
            w.body.freeze_left = left;
            assert_eq!(w.body.actor_frozen(), a, "actor @ {left:?}");
            assert_eq!(w.body.scene_frozen(), c, "scene @ {left:?}");
            assert_eq!(w.body.shots_frozen(), b, "shots @ {left:?}");
        }
    }

    /// P6：新字段必须进校验和（derive 默认全量入，本条是它的可观测面）。
    #[test]
    fn freeze_left_enters_the_checksum() {
        let mut w = crate::step::World::new(1);
        let base = w.checksum();
        w.body.freeze_left[0] = 1;
        assert_ne!(w.checksum(), base, "freeze_left 必须入校验和");
    }
}
