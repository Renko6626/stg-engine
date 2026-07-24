//! ECL 表层语言内建函数表 + `$` 引擎变量表（M1.9 T2）——**单一权威**：`lang::typeck` 消费本表
//! 做参数型/返回型校验，T3 codegen 消费本表拿 syscall 号/VM op 号发射指令（本模块文档即
//! 契约，T3 不得另起一张表）。号表来源：`crates/stg-core/src/ecl/syscall.rs` 常量 +
//! `docs/ecl-ops.md`（syscall）/`crates/stg-core/src/ecl/ops.rs`（`sin`/`cos` 例外，见下）。
//!
//! ## 与计划核心接口块的一处必要出入：`Builtin` 字段形状
//!
//! 计划给的示意是 `Builtin { name, syscall: u16, params: &'static [Ty], ret: Option<Ty> }`，
//! 但计划正文自己也预见了偏离的必要性——`fire` 的 `xf`/`task` 两个参数是**标识符参数**
//! （xformdef/sub 名字，编译期解析，不是求值表达式），逼 `params` 的元素类型从纯 `Ty` 扩成
//! [`ParamKind`]。顺带处理另一处出入：`sin`/`cos` 底层不是 syscall 派发（`OP_SYS <no>`），
//! 而是直接对应一条 VM op（`OP_SINB`/`OP_COSB`，见 `ecl::ops.rs`，30 值域族，非 60 值域族的
//! `OP_SYS`）——若硬塞进同一个 `syscall: u16` 字段会让 T3 误当成 syscall 号发 `OP_SYS 32`
//! （根本不存在的 syscall，运行期会 Fault）。加一个 `is_op: bool` 旗标消歧：`true` 时
//! `syscall` 字段其实装的是 VM op 码本身，T3 需要直接发那条 op（不套 `OP_SYS` 壳）；v1 只有
//! `sin`/`cos` 走这条支路，其余全部 `is_op=false` 正常 syscall 派发。
//!
//! ## `global(n)` 为何在本表（C16 复审修复，曾经不在）
//!
//! `global(n)` 曾是 parser 特判出的专属 AST 节点，绕开本表、也绕开 `check_call` 的调用解析——
//! 代价是它永远轮不到"先查 subs、再查 builtins"这条顺序：一个用户声明的同名
//! `sub global(...)` 会静默编译进镜像却永远调不到，没有任何错误或警告（follow-ups.md C16）。
//! 现在 `global(n)` 是普通表项（见下方 `syscall::SYS_GET_VAR`），与 `set_global(n, v)` 完全
//! 对称——两者都走 `Expr::Call` → `check_call`，都会被同名 sub 正常遮蔽，不再有特权通道。

use crate::lang::ast::{EngVar, Ty};
use stg_core::ecl::ops::{OP_COSB, OP_SINB};
use stg_core::ecl::syscall;

/// 一个内建函数的参数位期望——大多数是"求值表达式，判型为某个 `Ty`"（[`ParamKind::Val`]），
/// `fire` 的 `xf`/`task` 两位例外：语法上是裸标识符（xformdef 名 / sub 名）或字面量 `none`，
/// 编译期直接解析，不参与求值型检查（[`ParamKind::XformRef`]/[`ParamKind::SubRef`]，
/// `lang::typeck` 据此走不同的参数校验分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// 普通求值参数，期望类型 `Ty`（无隐式转换，与二元运算同一纪律）。
    Val(Ty),
    /// `xformdef` 名或 `none`（丙方案 `fire` 的 xform 引用参数）。
    XformRef,
    /// `sub` 名或 `none`（丙方案 `fire` 的任务脚本引用参数）。
    SubRef,
    /// 裸载荷参数（通道 B `emit_req` 六个 args 位）：接受 int/fx/angle 任意**良型**表达式，
    /// codegen 与 `Val` 同路径求值入栈、原样发射（VM 栈本就是裸 i32，零转换指令）——语义
    /// 镜像 `RenderReq.args` 的不透明本质：fx 过 Q16.16 raw、angle 过 BAM raw、int 原样。
    RawVal,
}

/// 内建函数的底层派发方式：绝大多数走 syscall 号表（`OP_SYS <no>`），`sin`/`cos` 是仅有的
/// 例外——直接对应一条 VM 算术 op（`OP_SINB`/`OP_COSB`），见模块文档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builtin {
    pub name: &'static str,
    /// `is_op=false`：`ecl::syscall::SYS_*` 号（T3 发 `OP_SYS <syscall>`）。
    /// `is_op=true`：VM op 码本身（T3 直接发这条 op，不套 `OP_SYS`）——v1 仅 `sin`/`cos`。
    pub syscall: u16,
    pub is_op: bool,
    pub params: &'static [ParamKind],
    /// `None` = 无返回值（值消费检查据此判断该调用能不能出现在表达式位置，见 `lang::typeck`）。
    pub ret: Option<Ty>,
    /// 人可读说明（编辑体验刀：hover/文档生成消费，见 `pub fn all()`）——逐条对照
    /// `ecl::syscall.rs` 对应 `sys_*` 函数核实过，出入以源码为准记入本刀报告，不是
    /// 简单转录 spec 草稿。永不为空（`all_builtins_have_doc_and_matching_param_names`
    /// 单测钉死）。
    pub doc: &'static str,
    /// 参数名（与 `params` 等长，一一对应；供签名提示/hover 消费）。`fire`/`spell_begin`
    /// 的 `Xf`/`Sub` 位置也占一个名字（如 `xf`/`task`/`pattern`），与 `Val`/`RawVal` 位置
    /// 同等对待——供参考的都是"这一位在表层调用里怎么称呼"，不区分底层求值方式。
    pub param_names: &'static [&'static str],
}

use ParamKind::{RawVal, SubRef as Sub, Val, XformRef as Xf};
use Ty::{Angle, Fx, Int};

/// v1.1 内建函数全集（源码序即本表序——`lookup` 线性扫描，条目 <30、无序容器无必要）。
const BUILTINS: &[Builtin] = &[
    // ── 创建/世界变更（syscall 2x）───────────────────────────────────────
    Builtin {
        name: "fire",
        syscall: syscall::SYS_CREATE_BULLET,
        is_op: false,
        // 丙方案 8 参 syscall 的表层化：xf/task 两位标识符参数收窄成 (off,cnt)/script，
        // T3 codegen 时机负责展开——本表只钉表层可见的 7 位。
        params: &[Val(Int), Val(Fx), Val(Fx), Val(Fx), Val(Angle), Xf, Sub],
        ret: Some(Int),
        doc: "发一颗弹;appearance 查外观表(越界 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1",
        param_names: &["appearance", "x", "y", "speed", "angle", "xf", "task"],
    },
    Builtin {
        name: "batch",
        syscall: syscall::SYS_CREATE_BULLETS_BATCH,
        is_op: false,
        params: &[
            Val(Int),
            Val(Fx),
            Val(Fx),
            Val(Int),
            Val(Angle),
            Val(Angle),
            Val(Int),
            Val(Fx),
            Val(Fx),
        ],
        ret: Some(Int),
        doc: "N-way 批量发环;返实际创建数",
        param_names: &[
            "appearance",
            "x",
            "y",
            "n_angle",
            "angle0",
            "angle_step",
            "n_speed",
            "speed0",
            "speed_step",
        ],
    },
    Builtin {
        name: "spawn_enemy",
        syscall: syscall::SYS_SPAWN_ENEMY,
        is_op: false,
        params: &[Val(Fx), Val(Fx), Val(Int), Val(Int), Val(Int)],
        ret: Some(Int),
        doc: "造敌;sprite 固定 0、判定 12/16 默认;返敌句柄,失败 -1",
        param_names: &["x", "y", "hp", "drop_table", "score"],
    },
    Builtin {
        name: "drop_item",
        syscall: syscall::SYS_DROP_ITEM,
        is_op: false,
        params: &[Val(Fx), Val(Fx), Val(Int)],
        ret: Some(Int),
        doc: "掉一颗道具(带随机喷发速度,消耗模拟 RNG);返句柄,失败 -1",
        param_names: &["x", "y", "item_type"],
    },
    Builtin {
        name: "move_to",
        syscall: syscall::SYS_MOVE_ENEMY_TO,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx), Val(Int)],
        ret: None,
        // 核对纠偏（见本刀报告）：草稿表原写 `enemy,x,y,dur`——但 `sys_move_enemy_to`
        // 里 `self_enemy_handle` 是从 self owner 取的，根本不占栈位；4 参逆序弹出实际是
        // `easing, y, x, dur`（源码注释原文），正序即 `dur, x, y, easing`。move_to 与
        // 下方弹 setter 族不同：它没有"占位 handle 参数"，4 位全部真实参与求值/入栈。
        doc: "敌自身(self owner 非 ENEMY → Fault)按 easing 缓动、dur 帧内平移到 (x,y);四参数皆真实压栈(不同于下方弹 setter 族的占位 handle 首参)",
        param_names: &["dur", "x", "y", "easing"],
    },
    Builtin {
        name: "boss_set",
        syscall: syscall::SYS_BOSS_SET,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Int), Val(Int), Val(Int), Val(Int)],
        ret: None,
        // 核对纠偏（见本刀报告）：草稿表首参写 `enemy`——但 `enemy` 字段是从 self owner
        // 取的（非 ENEMY → `EnemyHandle::NULL`，不 Fault），并不占栈位；6 参逆序弹出实际
        // 是 `active, phase_left, timer_frames, spell_id, hp_ratio, slot`，正序即
        // `slot, hp_ratio, spell_id, timer_frames, phase_left, active`（源码注释原文）。
        doc: "整槽写 boss_ui 公告板(脚本写/UI 读);enemy 字段取自 self owner(非 ENEMY → NULL,不 Fault);符卡 active 期 enemy/spell_id/timer_frames/hp_ratio 由引擎逐帧自动覆写,phase_left 不受影响仍归脚本",
        param_names: &[
            "slot",
            "hp_ratio",
            "spell_id",
            "timer_frames",
            "phase_left",
            "active",
        ],
    },
    Builtin {
        name: "pulse_signal",
        syscall: syscall::SYS_PULSE_SIGNAL,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        // 核对纠偏（见本刀报告）：草稿表 doc 写"唤醒 wait_signal 中的弹任务"——但
        // `WAIT_SIGNAL` 是弹变换(xform)序列里的一个 op（docs/xform-ops.md #51），由相位 4
        // `run_transforms` 消费，不是 ECL 任务/协程；"弹任务"一词会与 ECL task 混淆。
        doc: "脉冲一条信号通道(边沿语义,仅当帧有效);放行处于弹变换 WAIT_SIGNAL 停驻态的弹(非 ECL 任务)",
        param_names: &["channel"],
    },
    Builtin {
        name: "emit_req",
        syscall: syscall::SYS_EMIT_REQ,
        is_op: false,
        // 固定 7 参（不足位作者手补 0）；id 位钉 Int，六载荷位 RawVal（spec §2.6）
        params: &[Val(Int), RawVal, RawVal, RawVal, RawVal, RawVal, RawVal],
        ret: None,
        doc: "通道 B 渲染请求;void 只能裸语句;args 裸载荷(fx 过 raw/angle 过 BAM/int 原样)",
        param_names: &["id", "a0", "a1", "a2", "a3", "a4", "a5"],
    },
    // ── 读/杂项 ─────────────────────────────────────────────────────────
    Builtin {
        name: "rand",
        syscall: syscall::SYS_RAND_RANGE,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
        doc: "模拟 RNG 均匀 [0,n);确定性,随快照回卷",
        param_names: &["n"],
    },
    Builtin {
        name: "global",
        syscall: syscall::SYS_GET_VAR,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
        doc: "读 globals 槽(GVAR_RANK=0 为难度)",
        param_names: &["slot"],
    },
    Builtin {
        name: "set_global",
        syscall: syscall::SYS_SET_VAR,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "写 globals 槽;系统段(slot<16)脚本写为 no-op+计数,不 Fault(GVAR_RANK=0 建议脚本只读)",
        param_names: &["slot", "value"],
    },
    Builtin {
        name: "aim_player",
        syscall: syscall::SYS_AIM_PLAYER_ANGLE,
        is_op: false,
        params: &[],
        ret: Some(Angle),
        doc: "自身(敌/弹属主)指向自机的 BAM 角",
        param_names: &[],
    },
    Builtin {
        name: "sin",
        syscall: OP_SINB as u16,
        is_op: true,
        params: &[Val(Angle)],
        ret: Some(Fx),
        doc: "查表三角,返 fx(VM op 直发,非 syscall)",
        param_names: &["angle"],
    },
    Builtin {
        name: "cos",
        syscall: OP_COSB as u16,
        is_op: true,
        params: &[Val(Angle)],
        ret: Some(Fx),
        doc: "查表三角,返 fx(VM op 直发,非 syscall)",
        param_names: &["angle"],
    },
    // ── 弹 setter 族九连（syscall 3x；按 syscall.rs/motion.rs 顺序编号；handle:int 首参，
    // 见 plan 核心接口块——VM 侧 setter 语义实取 self owner，handle 参数的落地方式留 T3
    // 定，T2 只钉表层签名，见本刀报告"contract notes for T3"）───────────────────────
    //
    // 核对纠偏（见本刀报告，九条 + move_to 共十条同类修正）：草稿表逐条 doc 写
    // "(坏句柄 no-op+计数,下同)"——但 `syscall.rs` 模块文档"误用策略拍板"明文：
    // `self_bullet_handle`（`move_enemy_to`/弹 setter族/`aim_player_angle` 同款）owner
    // 类型不符是**脚本作者违约 → Fault**（响亮报错），不是静默 no-op+计数（no-op+计数是
    // `set_var`/`drop_item`/`emit_req` 一类"参数值域违规"的处置，两码事）。且首位
    // "handle" 从未真正被 `dispatch` 读取——`codegen::gen_builtin_call` 对这九个名字求值
    // 后立即 `pop()` 丢弃（`is_self_bullet_setter`），实际生效对象恒是 self owner；
    // "坏句柄"这个说法本身就不成立（没有"句柄值"参与判定）。
    Builtin {
        name: "set_speed",
        syscall: syscall::SYS_SET_BULLET_SPEED,
        is_op: false,
        params: &[Val(Int), Val(Fx)],
        ret: None,
        doc: "弹 setter:改速率;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定",
        param_names: &["handle", "speed"],
    },
    Builtin {
        name: "set_angle",
        syscall: syscall::SYS_SET_BULLET_ANGLE,
        is_op: false,
        params: &[Val(Int), Val(Angle)],
        ret: None,
        doc: "弹 setter:改方向;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定",
        param_names: &["handle", "angle"],
    },
    Builtin {
        name: "turn",
        syscall: syscall::SYS_TURN_BULLET,
        is_op: false,
        params: &[Val(Int), Val(Angle)],
        ret: None,
        doc: "弹 setter:转向增量;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定",
        param_names: &["handle", "delta"],
    },
    Builtin {
        name: "set_vel",
        syscall: syscall::SYS_SET_BULLET_VEL,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
        doc: "弹 setter:直设速度向量;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定",
        param_names: &["handle", "vx", "vy"],
    },
    Builtin {
        name: "set_ang_vel",
        syscall: syscall::SYS_SET_BULLET_ANG_VEL,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "弹 setter:角速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(POLAR_FX:每帧 angle+=w)",
        param_names: &["handle", "w"],
    },
    Builtin {
        name: "set_accel",
        syscall: syscall::SYS_SET_BULLET_ACCEL,
        is_op: false,
        params: &[Val(Int), Val(Fx)],
        ret: None,
        doc: "弹 setter:切向加速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(POLAR_FX:每帧 speed+=a)",
        param_names: &["handle", "a"],
    },
    Builtin {
        name: "set_gravity",
        syscall: syscall::SYS_SET_BULLET_GRAVITY,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
        doc: "弹 setter:直角加速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(CART_FX:每帧 v+=(gx,gy);与 POLAR_FX 互斥)",
        param_names: &["handle", "gx", "gy"],
    },
    Builtin {
        name: "stop_fx",
        syscall: syscall::SYS_STOP_BULLET_FX,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "弹 setter:停连续效果;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(清 POLAR_FX/CART_FX 连续效果)",
        param_names: &["handle"],
    },
    Builtin {
        name: "aim_at_player",
        syscall: syscall::SYS_AIM_BULLET_AT_PLAYER,
        is_op: false,
        params: &[Val(Int), Val(Angle)],
        ret: None,
        doc: "弹 setter:指向自机方向再加 offset 偏移角;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定",
        param_names: &["handle", "offset"],
    },
    // ── 符卡计器（syscall 28/29/11；符卡机构 spec 2026-07-24 §5）─────────────
    Builtin {
        name: "spell_begin",
        syscall: syscall::SYS_SPELL_BEGIN,
        is_op: false,
        // 第三位 `pattern` 是模式 sub 引用（`Sub`——同 `fire` 的 `task` 参同款
        // `ParamKind::SubRef`：sub 名或 `none`，编译期解析，不收求值表达式）。
        params: &[
            Val(Int),
            Val(Int),
            Sub,
            Val(Int),
            Val(Int),
            Val(Int),
            Val(Int),
        ],
        ret: None,
        doc: "开卡:绑 boss/血线/计时/计分,spawn pattern 为卡绑定模式任务(随卡生死)",
        param_names: &[
            "slot",
            "spell_id",
            "pattern",
            "time_limit",
            "bonus0",
            "flags",
            "hp_threshold",
        ],
    },
    Builtin {
        name: "spell_end",
        syscall: syscall::SYS_SPELL_END,
        is_op: false,
        params: &[],
        ret: None,
        doc: "手动收卡(取卡按血线自动判,通常不需要)",
        param_names: &[],
    },
    Builtin {
        name: "spell_timer",
        syscall: syscall::SYS_SPELL_TIMER,
        is_op: false,
        params: &[],
        ret: Some(Int),
        doc: "当前卡剩余帧数",
        param_names: &[],
    },
];

/// 按名字查内建函数（线性扫描；表 <30 项，`lang::typeck` 每次 `Call` 判型调用一次）。
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// 全量导出（编辑体验刀：`gen-ecl-meta`/VS Code 扩展/文档生成的单一真相源——不得另起
/// 一张手抄表，见模块文档"与计划核心接口块的一处必要出入"）。源码序即导出序（同 `lookup`
/// 的线性扫描序），不做任何排序/过滤。
pub fn all() -> &'static [Builtin] {
    BUILTINS
}

/// `$` 引擎变量的 syscall 号 + 判型（拍板 6 的 v1 白名单 8 个；`lang::parse` 已把 `$name` 解析
/// 成 [`EngVar`] 枚举，这里不需要再按字符串查——直接穷尽 `match`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngVarInfo {
    pub syscall: u16,
    pub ty: Ty,
}

pub fn engine_var_info(ev: EngVar) -> EngVarInfo {
    let (syscall, ty) = match ev {
        EngVar::Frame => (syscall::SYS_FRAME, Int),
        EngVar::PlayerX => (syscall::SYS_PLAYER_X, Fx),
        EngVar::PlayerY => (syscall::SYS_PLAYER_Y, Fx),
        EngVar::SelfX => (syscall::SYS_SELF_X, Fx),
        EngVar::SelfY => (syscall::SYS_SELF_Y, Fx),
        EngVar::SelfHp => (syscall::SYS_SELF_HP, Int),
        EngVar::SelfHpMax => (syscall::SYS_SELF_HP_MAX, Int),
        EngVar::SelfAge => (syscall::SYS_SELF_AGE, Int),
    };
    EngVarInfo { syscall, ty }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 编辑体验刀:元数据完备——每条 builtin 有非空 doc,param_names 与 params 等长。
    #[test]
    fn all_builtins_have_doc_and_matching_param_names() {
        for b in all() {
            assert!(!b.doc.is_empty(), "{} 缺 doc", b.name);
            assert_eq!(
                b.param_names.len(),
                b.params.len(),
                "{} 参数名/参数型不等长",
                b.name
            );
        }
    }

    #[test]
    fn lookup_finds_every_documented_builtin_by_name() {
        let names = [
            "fire",
            "batch",
            "spawn_enemy",
            "drop_item",
            "move_to",
            "boss_set",
            "pulse_signal",
            "emit_req",
            "rand",
            "global",
            "set_global",
            "aim_player",
            "sin",
            "cos",
            "set_speed",
            "set_angle",
            "turn",
            "set_vel",
            "set_ang_vel",
            "set_accel",
            "set_gravity",
            "stop_fx",
            "aim_at_player",
            "spell_begin",
            "spell_end",
            "spell_timer",
        ];
        for n in names {
            assert!(lookup(n).is_some(), "内建函数 '{n}' 应在表中");
        }
        assert_eq!(BUILTINS.len(), names.len(), "表内条目数应与穷举名单一致");
    }

    #[test]
    fn lookup_unknown_name_is_none() {
        assert!(lookup("no_such_builtin").is_none());
    }

    #[test]
    fn global_signature_matches_set_global_symmetrically() {
        let g = lookup("global").expect("global(n) 是普通 builtin 表项（C16 复审修复）");
        assert_eq!(g.ret, Some(Int));
        assert_eq!(g.params, &[Val(Int)]);
        assert_eq!(g.syscall, syscall::SYS_GET_VAR);
        assert!(!g.is_op);
    }

    #[test]
    fn fire_signature_matches_plan_shape() {
        let b = lookup("fire").unwrap();
        assert_eq!(b.ret, Some(Int));
        assert_eq!(
            b.params,
            &[Val(Int), Val(Fx), Val(Fx), Val(Fx), Val(Angle), Xf, Sub]
        );
        assert_eq!(b.syscall, syscall::SYS_CREATE_BULLET);
        assert!(!b.is_op);
    }

    #[test]
    fn sin_cos_dispatch_via_raw_vm_op_not_syscall() {
        let sin = lookup("sin").unwrap();
        assert!(sin.is_op, "sin 走 OP_SINB 直发，不是 syscall 派发");
        assert_eq!(sin.syscall, OP_SINB as u16);
        let cos = lookup("cos").unwrap();
        assert!(cos.is_op);
        assert_eq!(cos.syscall, OP_COSB as u16);
    }

    #[test]
    fn void_builtins_have_none_return_type() {
        for n in [
            "move_to",
            "boss_set",
            "pulse_signal",
            "emit_req",
            "set_global",
            "set_speed",
            "set_angle",
            "turn",
            "set_vel",
            "set_ang_vel",
            "set_accel",
            "set_gravity",
            "stop_fx",
            "aim_at_player",
            "spell_begin",
            "spell_end",
        ] {
            assert_eq!(lookup(n).unwrap().ret, None, "'{n}' 应无返回值");
        }
    }

    #[test]
    fn spell_begin_signature_matches_spec_shape() {
        let b = lookup("spell_begin").expect("spell_begin 应在表中");
        assert_eq!(
            b.params,
            &[
                Val(Int),
                Val(Int),
                Sub,
                Val(Int),
                Val(Int),
                Val(Int),
                Val(Int)
            ],
            "spell_begin 第三位应为 SubRef（同 fire 的 task 参同款）"
        );
        assert_eq!(b.ret, None);
        assert_eq!(b.syscall, syscall::SYS_SPELL_BEGIN);
        assert!(!b.is_op);
    }

    #[test]
    fn spell_timer_has_int_return_and_no_params() {
        let b = lookup("spell_timer").expect("spell_timer 应在表中");
        assert_eq!(b.params, &[]);
        assert_eq!(b.ret, Some(Int));
        assert_eq!(b.syscall, syscall::SYS_SPELL_TIMER);
    }

    #[test]
    fn bullet_setter_family_takes_handle_int_as_first_param() {
        for n in [
            "set_speed",
            "set_angle",
            "turn",
            "set_vel",
            "set_ang_vel",
            "set_accel",
            "set_gravity",
            "stop_fx",
            "aim_at_player",
        ] {
            let b = lookup(n).unwrap();
            assert_eq!(
                b.params.first(),
                Some(&Val(Int)),
                "'{n}' 首参应为 handle:int"
            );
        }
    }

    #[test]
    fn engine_var_table_matches_plan_syscall_numbers() {
        assert_eq!(
            engine_var_info(EngVar::Frame),
            EngVarInfo {
                syscall: syscall::SYS_FRAME,
                ty: Int
            }
        );
        assert_eq!(
            engine_var_info(EngVar::PlayerX),
            EngVarInfo {
                syscall: syscall::SYS_PLAYER_X,
                ty: Fx
            }
        );
        assert_eq!(
            engine_var_info(EngVar::SelfHp),
            EngVarInfo {
                syscall: syscall::SYS_SELF_HP,
                ty: Int
            }
        );
        assert_eq!(
            engine_var_info(EngVar::SelfHpMax),
            EngVarInfo {
                syscall: syscall::SYS_SELF_HP_MAX,
                ty: Int
            }
        );
        assert_eq!(
            engine_var_info(EngVar::SelfAge),
            EngVarInfo {
                syscall: syscall::SYS_SELF_AGE,
                ty: Int
            }
        );
    }
}
