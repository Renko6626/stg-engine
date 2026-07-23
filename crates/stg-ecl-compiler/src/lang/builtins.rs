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
    },
    Builtin {
        name: "spawn_enemy",
        syscall: syscall::SYS_SPAWN_ENEMY,
        is_op: false,
        params: &[Val(Fx), Val(Fx), Val(Int), Val(Int), Val(Int)],
        ret: Some(Int),
    },
    Builtin {
        name: "drop_item",
        syscall: syscall::SYS_DROP_ITEM,
        is_op: false,
        params: &[Val(Fx), Val(Fx), Val(Int)],
        ret: Some(Int),
    },
    Builtin {
        name: "move_to",
        syscall: syscall::SYS_MOVE_ENEMY_TO,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx), Val(Int)],
        ret: None,
    },
    Builtin {
        name: "boss_set",
        syscall: syscall::SYS_BOSS_SET,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Int), Val(Int), Val(Int), Val(Int)],
        ret: None,
    },
    Builtin {
        name: "pulse_signal",
        syscall: syscall::SYS_PULSE_SIGNAL,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
    },
    Builtin {
        name: "emit_req",
        syscall: syscall::SYS_EMIT_REQ,
        is_op: false,
        // 固定 7 参（不足位作者手补 0）；id 位钉 Int，六载荷位 RawVal（spec §2.6）
        params: &[Val(Int), RawVal, RawVal, RawVal, RawVal, RawVal, RawVal],
        ret: None,
    },
    // ── 读/杂项 ─────────────────────────────────────────────────────────
    Builtin {
        name: "rand",
        syscall: syscall::SYS_RAND_RANGE,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
    },
    Builtin {
        name: "global",
        syscall: syscall::SYS_GET_VAR,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
    },
    Builtin {
        name: "set_global",
        syscall: syscall::SYS_SET_VAR,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
    },
    Builtin {
        name: "aim_player",
        syscall: syscall::SYS_AIM_PLAYER_ANGLE,
        is_op: false,
        params: &[],
        ret: Some(Angle),
    },
    Builtin {
        name: "sin",
        syscall: OP_SINB as u16,
        is_op: true,
        params: &[Val(Angle)],
        ret: Some(Fx),
    },
    Builtin {
        name: "cos",
        syscall: OP_COSB as u16,
        is_op: true,
        params: &[Val(Angle)],
        ret: Some(Fx),
    },
    // ── 弹 setter 族九连（syscall 3x；按 syscall.rs/motion.rs 顺序编号；handle:int 首参，
    // 见 plan 核心接口块——VM 侧 setter 语义实取 self owner，handle 参数的落地方式留 T3
    // 定，T2 只钉表层签名，见本刀报告"contract notes for T3"）───────────────────────
    Builtin {
        name: "set_speed",
        syscall: syscall::SYS_SET_BULLET_SPEED,
        is_op: false,
        params: &[Val(Int), Val(Fx)],
        ret: None,
    },
    Builtin {
        name: "set_angle",
        syscall: syscall::SYS_SET_BULLET_ANGLE,
        is_op: false,
        params: &[Val(Int), Val(Angle)],
        ret: None,
    },
    Builtin {
        name: "turn",
        syscall: syscall::SYS_TURN_BULLET,
        is_op: false,
        params: &[Val(Int), Val(Angle)],
        ret: None,
    },
    Builtin {
        name: "set_vel",
        syscall: syscall::SYS_SET_BULLET_VEL,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
    },
    Builtin {
        name: "set_ang_vel",
        syscall: syscall::SYS_SET_BULLET_ANG_VEL,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
    },
    Builtin {
        name: "set_accel",
        syscall: syscall::SYS_SET_BULLET_ACCEL,
        is_op: false,
        params: &[Val(Int), Val(Fx)],
        ret: None,
    },
    Builtin {
        name: "set_gravity",
        syscall: syscall::SYS_SET_BULLET_GRAVITY,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
    },
    Builtin {
        name: "stop_fx",
        syscall: syscall::SYS_STOP_BULLET_FX,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
    },
    Builtin {
        name: "aim_at_player",
        syscall: syscall::SYS_AIM_BULLET_AT_PLAYER,
        is_op: false,
        params: &[Val(Int), Val(Angle)],
        ret: None,
    },
];

/// 按名字查内建函数（线性扫描；表 <30 项，`lang::typeck` 每次 `Call` 判型调用一次）。
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
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
        ] {
            assert_eq!(lookup(n).unwrap().ret, None, "'{n}' 应无返回值");
        }
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
