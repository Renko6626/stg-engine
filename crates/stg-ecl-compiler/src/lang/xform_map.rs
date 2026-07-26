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
//!
//! ## `Reserved`（C15 复审修复）
//!
//! `spawn_pattern`（op 60）编号已在 `docs/xform-ops.md` 钉死、也在 `stg_core::xform` 里
//! 定义了常量，但 `world/transform.rs` 的解释器还没实现它的分支——落进 `_` 兜底臂
//! 安全降级（P4-b：计 `contract_viol` + 终止序列，不 panic），运行期**安静地什么都不
//! 发生**，没有任何编译期或运行期的显眼信号。曾经的 [`lookup`] 把它当合法 op 收，
//! 编译器毫无异议地放行——这正是这颗雷的成因。`Reserved` 让"编号存在但解释器不接"这个
//! 状态在**单一权威表**里显式表达出来，调用方（`lang::slots`）据此在编译期就拒收，
//! 报错措辞明确点出"预留/尚未实现"，不与真正的拼写错误（走 `None`/"未知操作名"）混同。
//! 等 `world/transform.rs` 真正实现了这个 op，把这一行改回 `Op(...)` 即可，`lookup`
//! 的调用方不需要跟着改。

use stg_core::xform;

/// [`lookup`] 的结果：真正可用的 op，或已知但解释器未实现的预留编号。
pub(crate) enum XformOp {
    /// (op 字节, 实参个数, 物理槽数)。
    Op(u8, usize, usize),
    /// (op 字节, 物理槽数)——表层收 **2** 个常量参、**折叠进 `args[0]`** 的 op
    /// （颜色轴糖：`set_sprite(shape, color)`）。核心侧 `OP_SET_SPRITE` 只读
    /// `args[0]`——若按 `Op(_, 2, 1)` 直落 `args[1]`，颜色会被无声丢弃，故必须走本变体。
    OpFold2(u8, usize),
    /// (op 字节, 物理槽数)——表层收 **1** 个常量参，**`args[1]` 由编译器写入绑定表的
    /// `color_stride`**（`set_shape`/`set_color`，颜色轴刀 T7 的"部分设"两个 op）。
    /// 引擎据此在运行期把 sprite 拆回两维——世界层本身不需要表、也不认识"颜色"这回事
    /// （spec §4.4）。没有绑定表时无从得知 stride，codegen 必须报错而不是猜一个默认值。
    OpWithStride(u8, usize),
    /// 编号已知但运行期解释器未实现——编译期拒收（见模块文档）。
    Reserved,
}

pub(crate) fn lookup(name: &str) -> Option<XformOp> {
    use XformOp::Op;
    match name {
        "set_speed" => Some(Op(xform::OP_SET_SPEED, 1, 1)),
        "add_speed" => Some(Op(xform::OP_ADD_SPEED, 1, 1)),
        "step_speed" => Some(Op(xform::OP_STEP_SPEED, 2, 2)),
        "set_angle" => Some(Op(xform::OP_SET_ANGLE, 1, 1)),
        "turn" => Some(Op(xform::OP_TURN, 1, 1)),
        "aim_player" => Some(Op(xform::OP_AIM_PLAYER, 1, 1)),
        "step_angle" => Some(Op(xform::OP_STEP_ANGLE, 2, 2)),
        "set_sprite" => Some(XformOp::OpFold2(xform::OP_SET_SPRITE, 1)),
        "set_shape" => Some(XformOp::OpWithStride(xform::OP_SET_SHAPE, 1)),
        "set_color" => Some(XformOp::OpWithStride(xform::OP_SET_COLOR, 1)),
        "set_life" => Some(Op(xform::OP_SET_LIFE, 1, 1)),
        "set_ang_vel" => Some(Op(xform::OP_SET_ANG_VEL, 1, 1)),
        "set_accel" => Some(Op(xform::OP_SET_ACCEL, 1, 1)),
        "set_gravity" => Some(Op(xform::OP_SET_GRAVITY, 2, 1)),
        "stop_fx" => Some(Op(xform::OP_STOP_FX, 0, 1)),
        "wait_signal" => Some(Op(xform::OP_WAIT_SIGNAL, 1, 1)),
        "bounce_arm" => Some(Op(xform::OP_BOUNCE_ARM, 2, 1)),
        "spawn_pattern" => Some(XformOp::Reserved),
        _ => None,
    }
}

/// 一个 xformdef 的**物理**槽总数（未知/预留 op 名按 1 计——报错责任在 slots 趟，
/// 此处保证 sizing 不 panic 继续收集后续错误）。
pub(crate) fn physical_len(slots: &[crate::lang::ast::XfSlotLit]) -> usize {
    slots
        .iter()
        .map(|s| match lookup(&s.op_name) {
            Some(XformOp::Op(_, _, p))
            | Some(XformOp::OpFold2(_, p))
            | Some(XformOp::OpWithStride(_, p)) => p,
            _ => 1,
        })
        .sum()
}
