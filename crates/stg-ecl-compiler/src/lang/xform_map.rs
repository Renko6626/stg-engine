//! xformdef 操作名 → 字节码映射的**单一权威表**（slots 趟与 codegen 趟共用）。
//!
//! `physical` = 该 op 实际占用的 xform 槽数：STEP 族（`step_speed`/`step_angle`）是双槽
//! op——第二槽是引擎 scratch（`docs/xform-ops.md`："双槽 op 的第二槽是引擎 scratch"）。
//! **表层语言把 scratch 完全藏起来**：作者一条 `step_speed(...)` 即一条语义 op，
//! slots 趟按 `physical` 计区宽、codegen staging 自动补零 scratch 槽——不补则运行期
//! STEP 的 scratch 写入会**无声覆写下一条 authored 槽**（M1.9 T3 复审 Important，此表即修法）。
//!
//! `loop`/`END` 不开放：前者被词法占用且 LOOP 型控制流归任务弹（档三），后者由
//! 序列尾零填充天然表达。

use stg_core::xform;

/// (op 字节, 实参个数, 物理槽数)。
pub(crate) fn lookup(name: &str) -> Option<(u8, usize, usize)> {
    match name {
        "set_speed" => Some((xform::OP_SET_SPEED, 1, 1)),
        "add_speed" => Some((xform::OP_ADD_SPEED, 1, 1)),
        "step_speed" => Some((xform::OP_STEP_SPEED, 2, 2)),
        "set_angle" => Some((xform::OP_SET_ANGLE, 1, 1)),
        "turn" => Some((xform::OP_TURN, 1, 1)),
        "aim_player" => Some((xform::OP_AIM_PLAYER, 1, 1)),
        "step_angle" => Some((xform::OP_STEP_ANGLE, 2, 2)),
        "set_sprite" => Some((xform::OP_SET_SPRITE, 1, 1)),
        "set_life" => Some((xform::OP_SET_LIFE, 1, 1)),
        "set_ang_vel" => Some((xform::OP_SET_ANG_VEL, 1, 1)),
        "set_accel" => Some((xform::OP_SET_ACCEL, 1, 1)),
        "set_gravity" => Some((xform::OP_SET_GRAVITY, 2, 1)),
        "stop_fx" => Some((xform::OP_STOP_FX, 0, 1)),
        "wait_signal" => Some((xform::OP_WAIT_SIGNAL, 1, 1)),
        "bounce_arm" => Some((xform::OP_BOUNCE_ARM, 2, 1)),
        "spawn_pattern" => Some((xform::OP_SPAWN_PATTERN, 2, 1)),
        _ => None,
    }
}

/// 一个 xformdef 的**物理**槽总数（未知 op 名按 1 计——报错责任在 slots 趟，
/// 此处保证 sizing 不 panic 继续收集后续错误）。
pub(crate) fn physical_len(slots: &[crate::lang::ast::XfSlotLit]) -> usize {
    slots
        .iter()
        .map(|s| lookup(&s.op_name).map(|(_, _, p)| p).unwrap_or(1))
        .sum()
}
