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
        // 丙方案 8 参 syscall 的表层化：xf/task 两位标识符参数收窄成 (off,cnt)/script；
        // shape/color 两位反向——表层两参、codegen 折叠成单个 appearance 值（颜色轴刀）。
        params: &[
            Val(Int),
            Val(Int),
            Val(Fx),
            Val(Fx),
            Val(Fx),
            Val(Angle),
            Xf,
            Sub,
        ],
        ret: Some(Int),
        doc: "发一颗弹;shape/color 查外观表(越界/空格 编译期或 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1",
        param_names: &["shape", "color", "x", "y", "speed", "angle", "xf", "task"],
    },
    Builtin {
        name: "batch",
        syscall: syscall::SYS_CREATE_BULLETS_BATCH,
        is_op: false,
        // 首位同 `fire`：表层 shape/color 两参，codegen 折叠成单个 appearance 值。
        params: &[
            Val(Int),
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
        doc: "N-way 批量发环;shape/color 同 fire;返实际创建数",
        param_names: &[
            "shape",
            "color",
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
        // A5 乙案（append-only）：旧 5 参前缀不动，尾追 sprite（求值参）、task（`SubRef`，
        // 同 fire 第 7 参同构）。
        params: &[
            Val(Fx),
            Val(Fx),
            Val(Int),
            Val(Int),
            Val(Int),
            Val(Int),
            Sub,
        ],
        ret: Some(Int),
        doc: "造敌;判定 12/16 默认;task 为敌主任务 async sub 名或 none(owner=新敌;敌死任务亡,任务跑完敌也亡——静默退场,不掉道具不发死亡事件);返敌号(不透明值,别猜数值/别做算术;两个敌号相等 ⇒ 同一只敌),失败 -1",
        param_names: &["x", "y", "hp", "drop_table", "score", "sprite", "task"],
    },
    Builtin {
        name: "enemy_hp",
        syscall: syscall::SYS_ENEMY_HP,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
        doc: "查敌当前 hp;死/悬垂/越界句柄返 -1(P4-b;敌号带 generation,槽被另一只敌复用后旧号照样返 -1)——stage 编排等 boss 死用",
        param_names: &["handle"],
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
    // 敌人运动动词族刀（T4，2026-07-31）：对标 ZUN ECL `move` 400-447 族，四条 syscall
    // 是 `world/motion.rs` 四个 `set_enemy_*` 写 API 的薄封装。全部 self-only，参数逆序
    // 弹出（正序即文档序，同 move_to）。设计见 spec §4.1。
    Builtin {
        name: "move_vel",
        syscall: syscall::SYS_MOVE_VEL,
        is_op: false,
        params: &[Val(Int), Val(Angle), Val(Fx), Val(Int)],
        ret: None,
        doc: "敌自身(self owner 非 ENEMY → Fault)按 easing 在 dur 帧内把速度缓动到「angle 方向、speed 速率」;dur=0 = 立即设。**极坐标空间插值**(匀速扫弧,速率按曲线走)——要笛卡尔直线插值用 move_vel_xy",
        param_names: &["dur", "angle", "speed", "easing"],
    },
    Builtin {
        name: "move_vel_xy",
        syscall: syscall::SYS_MOVE_VEL_XY,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx), Val(Int)],
        ret: None,
        doc: "同 move_vel 但收笛卡尔分量,且 dur>0 时**在笛卡尔空间插值**(两分量各自线性插,中途速率会掉——线性缓动即恒定加速度);要匀速转向用 move_vel。保住一轴的写法:move_vel_xy(30, $self_vx, 4.0fx, 2)",
        param_names: &["dur", "vx", "vy", "easing"],
    },
    Builtin {
        name: "move_angle",
        syscall: syscall::SYS_MOVE_ANGLE,
        is_op: false,
        params: &[Val(Int), Val(Angle), Val(Int)],
        ret: None,
        doc: "只转向、速率一字不动;dur>0 走**最短弧**(350deg→10deg 走 +20deg 不走 -340deg)。相对转向:move_angle(60, $self_angle + 15deg, 3)",
        param_names: &["dur", "angle", "easing"],
    },
    Builtin {
        name: "move_speed",
        syscall: syscall::SYS_MOVE_SPEED,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Int)],
        ret: None,
        doc: "只调速、方向一字不动。相对加速:move_speed(30, $self_speed * 2.0fx, 2)",
        param_names: &["dur", "speed", "easing"],
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
    // 小清洗刀（2026-07-31）：核里现成的三件东西通电——CORDIC `atan2`、`isqrt∘len_sq`、
    // `world::nearest_enemy`（后者自 M0-13 起是死代码）。零新机制，只是脚本够得着了。
    Builtin {
        name: "atan2",
        syscall: syscall::SYS_ATAN2,
        is_op: false,
        params: &[Val(Fx), Val(Fx)],
        ret: Some(Angle),
        doc: "任意向量的方向角(整数 CORDIC,16 轮);参数序 (y, x) 同 libm;(0,0) 返 0 不报错;比 aim_player 通用——能瞄任意点",
        param_names: &["y", "x"],
    },
    Builtin {
        name: "dist",
        syscall: syscall::SYS_DIST,
        is_op: false,
        params: &[Val(Fx), Val(Fx)],
        ret: Some(Fx),
        doc: "向量 (dx,dy) 的模长(开根,不是平方);**不是两点距离**——两点距离自己减: dist(bx-ax, by-ay)",
        param_names: &["dx", "dy"],
    },
    Builtin {
        name: "nearest_enemy",
        syscall: syscall::SYS_NEAREST_ENEMY,
        is_op: false,
        params: &[Val(Fx), Val(Fx)],
        ret: Some(Int),
        doc: "离 (x,y) 最近的活敌(非 dying;并列取低索引);无敌返 -1;返的敌号与 spawn_enemy 同编码,可直接喂 enemy_alive/enemy_hp/enemy_x/enemy_y(带 generation,槽复用后旧号可辨);它已排除 dying,故拿到的号过几帧可能已变 dying——该重查而不是继续用",
        param_names: &["x", "y"],
    },
    // 敌坐标读口刀（2026-07-31）：上一刀通电 `nearest_enemy` 后暴露的断头路——拿得到敌号
    // 读不到坐标，"查最近的敌 → 朝它开火"接不通。数据本就在敌池里，缺的只是读口。
    Builtin {
        name: "enemy_x",
        syscall: syscall::SYS_ENEMY_X,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Fx),
        doc: "按敌号读 x;死/悬垂/越界/槽已被别的敌复用 → 返 0(**不是哨兵**——0 是合法坐标,先用 enemy_alive(e) == 1 探活再读)",
        param_names: &["handle"],
    },
    Builtin {
        name: "enemy_y",
        syscall: syscall::SYS_ENEMY_Y,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Fx),
        doc: "按敌号读 y;死/悬垂/越界/槽已被别的敌复用 → 返 0(同 enemy_x,先探活再读);配 enemy_x + atan2 即可朝任意敌开火",
        param_names: &["handle"],
    },
    // 探活读口刀（2026-07-31）：专用探活口，堵 `enemy_hp(e) != -1` 那条残余缝
    // （−1 同时是降级值和一个合法血量 ⇒ overkill 的活敌会被旧探针误判）。
    Builtin {
        name: "enemy_alive",
        syscall: syscall::SYS_ENEMY_ALIVE,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
        doc: "敌号是否仍指向**它当初那只敌**,返 1/0(探活首选,比 enemy_hp(e) != -1 稳——血量恰为 -1 的活敌不会被误判;敌号带 generation,槽被另一只敌复用后旧号返 0);**含正在死的敌**(判的是槽有效不是还能打)",
        param_names: &["handle"],
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
    // ── 表现锚点四字段（syscall 5x；整局流程刀 Task 2/3）─────────────────────
    Builtin {
        name: "add_score",
        syscall: syscall::SYS_ADD_SCORE,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "给自机记分:delta 允许负(扣分),饱和钳 [0,u64::MAX] 不回绕;关底 bonus/结算记账用",
        param_names: &["delta"],
    },
    Builtin {
        name: "bgm",
        syscall: syscall::SYS_BGM,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "声明当前 BGM:写世界锚点字段 bgm_id 并发 REQ_BGM;mark 跳入自动补偿最近声明(常量参)",
        param_names: &["id"],
    },
    Builtin {
        name: "bg",
        syscall: syscall::SYS_BG,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "声明当前背景:写锚点 bg_id 并发 REQ_BG;换背景隐含新的 phase 纪元(补偿细则见 ecl-lang)",
        param_names: &["id"],
    },
    Builtin {
        name: "bg_phase",
        syscall: syscall::SYS_BG_PHASE,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "声明背景演出段号:写 bg_phase 并自动盖 bg_phase_frame=当前帧,发 REQ_BG_PHASE;表现层按段内局部时间 seek",
        param_names: &["phase"],
    },
    // ── B19：全场清弹（整局流程刀 Task 2；关底转场用）─────────────────────────
    Builtin {
        name: "clear_bullets",
        syscall: syscall::SYS_CLEAR_BULLETS,
        is_op: false,
        params: &[],
        ret: None,
        doc: "全场清弹:铺一个覆盖全场、存活 1 帧的消弹区(复用 FieldPool),每颗被消的弹原位转一颗星星(M0-15);不给护盾帧",
        param_names: &[],
    },
    // ── B20：账面增量三件套（syscall 55/56/57）。只有 add_*、没有 set_*——绝对赋值场景
    //    已被 Loadout（开局装备）收编，是人类裁定，别"补全"（裁定详见 syscall.rs 号表注释）。
    Builtin {
        name: "add_lives",
        syscall: syscall::SYS_ADD_LIVES,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "增减残机:delta 允许负,双边钳 [0,255] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_",
        param_names: &["delta"],
    },
    Builtin {
        name: "add_bombs",
        syscall: syscall::SYS_ADD_BOMBS,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "增减 bomb 数:delta 允许负,双边钳 [0,255] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_",
        param_names: &["delta"],
    },
    Builtin {
        name: "add_power",
        syscall: syscall::SYS_ADD_POWER,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "增减火力:delta 允许负,双边钳 [0,POWER_MAX=400](即显示 4.00,不是 u16::MAX);开局初值走 Loadout",
        param_names: &["delta"],
    },
    // ── 敌人死亡效果（syscall 58-61；参照 ZUN ECL 506/507/509/561）───────────
    Builtin {
        name: "drop_clear",
        syscall: syscall::SYS_DROP_CLEAR,
        is_op: false,
        params: &[],
        ret: None,
        doc: "清空自身待掉落计数;self 必须是敌",
        param_names: &[],
    },
    Builtin {
        name: "drop_add",
        syscall: syscall::SYS_DROP_ADD,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "自身待掉落计数增量加 n 颗 type(只增不减,要清空用 drop_clear);计数上限 255 饱和",
        param_names: &["type", "n"],
    },
    Builtin {
        name: "drop_items",
        syscall: syscall::SYS_DROP_ITEMS,
        is_op: false,
        params: &[],
        ret: None,
        doc: "立刻撒出自身待掉落计数;**吐完不清空**(故 drop_items();die(); 掉双份);不加分不发死亡事件",
        param_names: &[],
    },
    Builtin {
        name: "die",
        syscall: syscall::SYS_DIE,
        is_op: false,
        params: &[],
        ret: None,
        doc: "就地阵亡:掉落+加分+死亡事件+死亡特效,并**立即终止本任务**(后续语句不执行)",
        param_names: &[],
    },
    // ── Shooter：预存发射参数集（syscall 62-76；参照 ZUN et* 族 600-641）───────
    //    `sh_reset` 重置编号槽 → 一堆以 `id` 打头的 setter 逐项配 → `sh_fire(id)` 开火。
    //    改一个字段再开一次火就是下一波。每任务 4 个槽（id ∈ 0..4），槽号越界一律 no-op+计数。
    //    **前 14 条只写字段、无副作用**：appearance 在册/xform 区间/sub 号在册的校验
    //    全部推迟到 `sh_fire` 那一刻（设参数时弹还不存在，没有可拒绝的对象）。
    Builtin {
        name: "sh_reset",
        syscall: syscall::SYS_SH_RESET,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "重置发射器槽 id 为默认(1×1 单发、无 xform/挂弹任务/请求)",
        param_names: &["id"],
    },
    Builtin {
        name: "sh_sprite",
        syscall: syscall::SYS_SH_SPRITE,
        is_op: false,
        // 颜色轴糖：表层 shape/color 两参，codegen 折叠成单个 appearance（同 fire/batch）。
        // **折叠起点是下标 1**（第 1 参是 id）——见 `fold_start`。
        params: &[Val(Int), Val(Int), Val(Int)],
        ret: None,
        doc: "设发射器的弹型与颜色;查外观表(越界/空格 编译期或 Fault)",
        param_names: &["id", "shape", "color"],
    },
    Builtin {
        name: "sh_offset",
        syscall: syscall::SYS_SH_OFFSET,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
        doc: "设出弹点**相对 owner** 的偏移;与 sh_offset_abs 写同一对字段,后写的赢(本条清绝对位标志)",
        param_names: &["id", "x", "y"],
    },
    Builtin {
        name: "sh_offset_abs",
        syscall: syscall::SYS_SH_OFFSET_ABS,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
        doc: "设出弹点的**绝对**坐标(不跟随 owner);与 sh_offset 写同一对字段,后写的赢",
        param_names: &["id", "x", "y"],
    },
    Builtin {
        name: "sh_offset_rad",
        syscall: syscall::SYS_SH_OFFSET_RAD,
        is_op: false,
        params: &[Val(Int), Val(Angle), Val(Fx)],
        ret: None,
        doc: "设出弹点的极坐标偏移;与 sh_offset/sh_offset_abs **永远叠加**,不是覆盖",
        param_names: &["id", "angle", "r"],
    },
    Builtin {
        name: "sh_dist",
        syscall: syscall::SYS_SH_DIST,
        is_op: false,
        params: &[Val(Int), Val(Fx)],
        ret: None,
        doc: "出生后沿**各自角度**把弹推出去的距离(逐颗方向不同,不是整体平移)",
        param_names: &["id", "d"],
    },
    Builtin {
        name: "sh_angle",
        syscall: syscall::SYS_SH_ANGLE,
        is_op: false,
        params: &[Val(Int), Val(Angle), Val(Angle)],
        ret: None,
        doc: "设基准角与逐弹角增量;开了 sh_aim 时 angle0 是相对自机方向的偏移,开了 sh_ring 时 step 转义成逐层偏移",
        param_names: &["id", "angle0", "step"],
    },
    Builtin {
        name: "sh_speed",
        syscall: syscall::SYS_SH_SPEED,
        is_op: false,
        params: &[Val(Int), Val(Fx), Val(Fx)],
        ret: None,
        doc: "设基准速度与逐层速度增量(层数 = sh_count 的 n_speed)",
        param_names: &["id", "speed0", "step"],
    },
    Builtin {
        name: "sh_count",
        syscall: syscall::SYS_SH_COUNT,
        is_op: false,
        params: &[Val(Int), Val(Int), Val(Int)],
        ret: None,
        doc: "设发弹阵列规模:角度向 n_angle 颗 × 速度向 n_speed 层;双边钳 [0,255] 不回绕",
        param_names: &["id", "n_angle", "n_speed"],
    },
    Builtin {
        name: "sh_aim",
        syscall: syscall::SYS_SH_AIM,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "开/关自机狙(on!=0 为开):开则 sh_angle 的 angle0 是相对自机方向的偏移,而非绝对方向",
        param_names: &["id", "on"],
    },
    Builtin {
        name: "sh_ring",
        syscall: syscall::SYS_SH_RING,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "开/关整周环(on!=0 为开):开则 n_angle 颗自动均分整周;关则是以基准方向为中心对称展开的 fan",
        param_names: &["id", "on"],
    },
    Builtin {
        name: "sh_xform",
        syscall: syscall::SYS_SH_XFORM,
        is_op: false,
        params: &[Val(Int), Xf],
        ret: None,
        doc: "给发射器挂 xformdef(名或 none);开火时每颗弹都带上",
        param_names: &["id", "xf"],
    },
    Builtin {
        name: "sh_task",
        syscall: syscall::SYS_SH_TASK,
        is_op: false,
        params: &[Val(Int), Sub],
        ret: None,
        doc: "给发射器挂弹任务 async sub(名或 none);开火时每颗弹都派一个,owner=该弹",
        param_names: &["id", "sub"],
    },
    Builtin {
        name: "sh_req",
        syscall: syscall::SYS_SH_REQ,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "设开火时顺带发的通道 B 请求 id(音效等);0 = 不发",
        param_names: &["id", "req_id"],
    },
    Builtin {
        name: "sh_fire",
        syscall: syscall::SYS_SH_FIRE,
        is_op: false,
        params: &[Val(Int)],
        // **无返回值是人类裁定 D-8**：本语言要求值必须消费,有返回值就得写
        // `_ = sh_fire(0);`,而开火是循环里最高频的语句。别"补全"成返回实发数。
        ret: None,
        doc: "用发射器槽 id 的参数开火;无返回值;池满走 P4-a 计数",
        param_names: &["id"],
    },
];

/// 按名字查内建函数（线性扫描；表 <30 项，`lang::typeck` 每次 `Call` 判型调用一次）。
pub fn lookup(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// "弹型 + 颜色"两参糖的**起始参数下标**（`None` = 该内建没有两参糖）——**单一权威**。
///
/// （曾另有一个 `folds_shape_color(name) -> bool` 的存在性投影。shooter 刀把两个消费者都
/// 改成问起点之后它只剩测试在调，且它答的恰是这道题**没用的那一半**——真正咬人的是起点
/// 填错、不是"有没有"，故随本刀删除。要判"有没有"写 `fold_start(n).is_some()`。）
///
/// `lang::typeck` 据此施加图集三判据（`lang::atlas`）、`lang::codegen` 据此把那两位折叠成
/// 单个 appearance 值。两处**必须问同一个函数**：从名单里掉出去都是静默事故——typeck 掉了
/// = 判据不施加（隐形弹重新可造）；codegen 掉了 = 给 8 参 syscall 压 9 个值，整条参数序列
/// 错位，只有间接信号能发现。
///
/// **起点不总是 0**：`fire`/`batch` 的 `shape`/`color` 是前两参，但
/// `sh_sprite(id, shape, color)` 的第 1 参是发射器槽号，折的是第 2、3 参。判据曾经硬编码
/// "下标 0"，加 `sh_sprite` 时必须改成按内建查——否则会把 `id` 和 `shape` 折在一起。
/// 起点与 [`BUILTINS`] 的参数名表由 `fold_start_matches_the_param_name_table` 双向钉死；
/// `fire`/`batch` 的产物逐字节不变另有 `fire_and_batch_bytecode_is_byte_for_byte_unchanged`
/// 押运。
pub fn fold_start(name: &str) -> Option<usize> {
    match name {
        "fire" | "batch" => Some(0),
        "sh_sprite" => Some(1),
        _ => None,
    }
}

/// 全量导出（编辑体验刀：`gen-ecl-meta`/VS Code 扩展/文档生成的单一真相源——不得另起
/// 一张手抄表，见模块文档"与计划核心接口块的一处必要出入"）。源码序即导出序（同 `lookup`
/// 的线性扫描序），不做任何排序/过滤。
pub fn all() -> &'static [Builtin] {
    BUILTINS
}

/// `$` 引擎变量的 syscall 号 + 判型（拍板 6 的 v1 白名单 8 个；敌人运动动词族刀
/// 2026-07-31 扩到 12 个；`lang::parse` 已把 `$name` 解析成 [`EngVar`] 枚举，
/// 这里不需要再按字符串查——直接穷尽 `match`）。
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
        EngVar::SelfVx => (syscall::SYS_SELF_VX, Fx),
        EngVar::SelfVy => (syscall::SYS_SELF_VY, Fx),
        EngVar::SelfSpeed => (syscall::SYS_SELF_SPEED, Fx),
        EngVar::SelfAngle => (syscall::SYS_SELF_ANGLE, Angle),
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
            "enemy_hp",
            "drop_item",
            "move_to",
            "move_vel",
            "move_vel_xy",
            "move_angle",
            "move_speed",
            "boss_set",
            "pulse_signal",
            "emit_req",
            "rand",
            "global",
            "set_global",
            "aim_player",
            "atan2",
            "dist",
            "nearest_enemy",
            "enemy_x",
            "enemy_y",
            "enemy_alive",
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
            "add_score",
            "bgm",
            "bg",
            "bg_phase",
            "clear_bullets",
            "add_lives",
            "add_bombs",
            "add_power",
            "drop_clear",
            "drop_add",
            "drop_items",
            "die",
            "sh_reset",
            "sh_sprite",
            "sh_offset",
            "sh_offset_abs",
            "sh_offset_rad",
            "sh_dist",
            "sh_angle",
            "sh_speed",
            "sh_count",
            "sh_aim",
            "sh_ring",
            "sh_xform",
            "sh_task",
            "sh_req",
            "sh_fire",
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
            &[
                Val(Int),
                Val(Int),
                Val(Fx),
                Val(Fx),
                Val(Fx),
                Val(Angle),
                Xf,
                Sub
            ]
        );
        assert_eq!(b.syscall, syscall::SYS_CREATE_BULLET);
        assert!(!b.is_op);
    }

    /// 颜色轴刀：`fire`/`batch` 头两位是 `shape`/`color`（**顺序**是契约——两位同为
    /// `Val(Int)`，对调不会有任何判型报错，只会静默发错弹型；同 `spawn_enemy` 先例）。
    #[test]
    fn fire_and_batch_lead_with_shape_then_color() {
        for n in ["fire", "batch"] {
            let b = lookup(n).unwrap();
            assert_eq!(b.param_names[0], "shape", "'{n}' 第 1 位");
            assert_eq!(b.param_names[1], "color", "'{n}' 第 2 位");
            assert!(
                matches!(b.params[0], Val(Int)) && matches!(b.params[1], Val(Int)),
                "'{n}' 头两位都必须是 Val(Int)"
            );
        }
    }

    /// 折叠谓词**与起点**跟参数名表**双向**一致：某相邻两位恰名为 `shape`/`color` ⟺
    /// [`fold_start`] 指向那一位。防三种漂移——加了两参糖却忘登记（判据不施加、codegen
    /// 不折叠），谓词里多写一个名字（给 syscall 多压一个值），或**起点填错**（`sh_sprite`
    /// 填 0 就会把 `id` 和 `shape` 折在一起，整条参数序列错位）。
    #[test]
    fn fold_start_matches_the_param_name_table() {
        for b in all() {
            let by_names = b
                .param_names
                .windows(2)
                .position(|w| w == ["shape", "color"]);
            assert_eq!(
                fold_start(b.name),
                by_names,
                "'{}' 的折叠起点与参数名表不一致",
                b.name
            );
        }
        assert_eq!(fold_start("fire"), Some(0));
        assert_eq!(fold_start("batch"), Some(0));
        assert_eq!(
            fold_start("sh_sprite"),
            Some(1),
            "第 1 参是 id，折的是 2/3 参"
        );
        assert_eq!(fold_start("spawn_enemy"), None, "sprite 位不是两参糖");
    }

    /// `batch` 的 10 位形状（头两位 shape/color，其余沿用 syscall 既有顺序）。
    #[test]
    fn batch_signature_matches_plan_shape() {
        let b = lookup("batch").unwrap();
        assert_eq!(
            b.params,
            &[
                Val(Int),
                Val(Int),
                Val(Fx),
                Val(Fx),
                Val(Int),
                Val(Angle),
                Val(Angle),
                Val(Int),
                Val(Fx),
                Val(Fx)
            ]
        );
        assert_eq!(b.ret, Some(Int));
        assert_eq!(b.syscall, syscall::SYS_CREATE_BULLETS_BATCH);
        assert!(!b.is_op);
    }

    /// A5 乙案（task-1 复审 Important-3）：`spawn_enemy` 7 位形状——`sprite` 在第 6 位
    /// （`Val(Int)`，求值参），`task` 在第 7 位（`Sub`，`SubRef` 同 `fire` 第 7 参同构）；
    /// 防 params 表两位对调静默错位（沿用 `fire_signature_matches_plan_shape` 先例）。
    #[test]
    fn spawn_enemy_signature_matches_plan_shape() {
        let b = lookup("spawn_enemy").expect("spawn_enemy 应在表中");
        assert_eq!(
            b.params,
            &[
                Val(Fx),
                Val(Fx),
                Val(Int),
                Val(Int),
                Val(Int),
                Val(Int),
                Sub
            ],
            "spawn_enemy 第 6 位应为 sprite:Val(Int)，第 7 位应为 task:SubRef"
        );
        assert_eq!(b.ret, Some(Int));
        assert_eq!(b.syscall, syscall::SYS_SPAWN_ENEMY);
        assert!(!b.is_op);
    }

    /// A5 补遗（task-1 复审 Important-3）：`enemy_hp` 单参 `handle:int`，返 `int`。
    #[test]
    fn enemy_hp_signature_matches_plan_shape() {
        let b = lookup("enemy_hp").expect("enemy_hp 应在表中");
        assert_eq!(b.params, &[Val(Int)]);
        assert_eq!(b.ret, Some(Int));
        assert_eq!(b.syscall, syscall::SYS_ENEMY_HP);
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
            "move_vel",
            "move_vel_xy",
            "move_angle",
            "move_speed",
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
            "add_score",
            "bgm",
            "bg",
            "bg_phase",
            "clear_bullets",
            "add_lives",
            "add_bombs",
            "add_power",
            "drop_clear",
            "drop_add",
            "drop_items",
            "die",
            "sh_reset",
            "sh_sprite",
            "sh_offset",
            "sh_offset_abs",
            "sh_offset_rad",
            "sh_dist",
            "sh_angle",
            "sh_speed",
            "sh_count",
            "sh_aim",
            "sh_ring",
            "sh_xform",
            "sh_task",
            "sh_req",
            "sh_fire",
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

    /// 小清洗刀（77-79）：三条新内建的**返回型**与 syscall 号。返回型是契约——
    /// `atan2` 返 `angle`（拿去喂 `fire` 的角度位不用 cast）、`dist` 返 `fx`、
    /// `nearest_enemy` 返 `int`（池 index）；写错任何一个都会让作者被迫加位穿透 cast。
    #[test]
    fn math_and_query_builtins_return_types_and_syscall_numbers() {
        let a = lookup("atan2").expect("atan2 应在表中");
        assert_eq!(a.params, &[Val(Fx), Val(Fx)], "atan2 两参都是 fx");
        assert_eq!(a.param_names, &["y", "x"], "参数序是 (y, x)，同 libm 惯例");
        assert_eq!(a.ret, Some(Angle));
        assert_eq!(a.syscall, syscall::SYS_ATAN2);
        assert!(!a.is_op);

        let d = lookup("dist").expect("dist 应在表中");
        assert_eq!(d.params, &[Val(Fx), Val(Fx)]);
        assert_eq!(d.ret, Some(Fx));
        assert_eq!(d.syscall, syscall::SYS_DIST);
        assert!(!d.is_op);

        let n = lookup("nearest_enemy").expect("nearest_enemy 应在表中");
        assert_eq!(n.params, &[Val(Fx), Val(Fx)]);
        assert_eq!(n.ret, Some(Int), "返的是池 index，不是 fx");
        assert_eq!(n.syscall, syscall::SYS_NEAREST_ENEMY);
        assert!(!n.is_op);
    }

    /// 敌坐标读口刀（80/81）：**返回型必须是 `fx`**——它们存在的全部理由就是拿去减、
    /// 喂 `atan2`/`dist`，返 `int` 会让作者每处都补一记穿透 cast。号也逐条钉死，防
    /// 80/81 两条派发臂在表里写反（两条内建同签名，写反了 typeck 一声不吭）。
    #[test]
    fn enemy_pos_builtins_return_fx_and_carry_their_own_syscall_numbers() {
        let x = lookup("enemy_x").expect("enemy_x 应在表中");
        assert_eq!(x.params, &[Val(Int)], "1 参：池 index");
        assert_eq!(x.param_names, &["handle"]);
        assert_eq!(x.ret, Some(Fx), "返 fx（直接可减/可喂 atan2）");
        assert_eq!(x.syscall, syscall::SYS_ENEMY_X);
        assert!(!x.is_op);

        let y = lookup("enemy_y").expect("enemy_y 应在表中");
        assert_eq!(y.params, &[Val(Int)]);
        assert_eq!(y.ret, Some(Fx));
        assert_eq!(y.syscall, syscall::SYS_ENEMY_Y);
        assert!(!y.is_op);
        assert_ne!(x.syscall, y.syscall, "两条不能共用一个号");
    }

    /// 探活读口刀（82）：`enemy_alive` 返 **`int`**（1/0 的布尔面孔，直接进 `if` 条件），
    /// **不是** `fx`——返 `fx` 会让 `enemy_alive(e) == 1` 这个招牌写法判型失败。
    #[test]
    fn enemy_alive_builtin_returns_int_and_carries_its_own_syscall_number() {
        let a = lookup("enemy_alive").expect("enemy_alive 应在表中");
        assert_eq!(a.params, &[Val(Int)], "1 参：池 index");
        assert_eq!(a.param_names, &["handle"]);
        assert_eq!(a.ret, Some(Int), "返 1/0（可直接进 if 条件）");
        assert_eq!(a.syscall, syscall::SYS_ENEMY_ALIVE);
        assert!(!a.is_op);
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
