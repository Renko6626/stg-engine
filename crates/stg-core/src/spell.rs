//! 符卡计器的 POD 数据槽与常量（spec 2026-07-24）。
//! `SpellSlot` 入 WorldBody（checksum/copy_into/SaveBytes 三件套）；
//! 记账与结算逻辑位于 `world/spell.rs`，换卡、阶段切换及弹幕行为仍归脚本。

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
