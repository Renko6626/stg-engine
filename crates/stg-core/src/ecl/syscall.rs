//! ecl/syscall.rs —— syscall 号表 v1（冻结起点）+ 绑定层派发（M1 T3）。
//!
//! **参数传递约定**（spec/plan 既定）：脚本按声明顺序**正序压栈**（param1, param2, ...,
//! paramN）；栈是 LIFO，故 `dispatch` 按**逆序弹出**（paramN 先弹 ... param1 后弹）——
//! 这与普通函数调用栈约定一致，不是额外规则。读类 syscall（`frame`/`player_x`/`self_x`/...）
//! 执行后压回 1 个返回值；创建类（`create_bullet`/`create_bullets_batch`/`spawn_enemy`/
//! `drop_item`）压 1 个返回值（句柄 index 转 i32，失败 -1；批量创建压实发数）；写类
//! （`set_var`/`move_enemy_to`/`boss_set`/`pulse_signal`/弹 setter 族）不压值。
//!
//! **坏号/参数不足（栈下溢）→ Fault**：`dispatch` 返回 `Result<(), u8>`，`Err(code)` 由
//! `vm::exec` 的 `OP_SYS` 分支转成 `Exec::Fault(code)`；坏 syscall 号统一复用
//! `vm::FAULT_BAD_OP`（同 `OP_SPAWN` 坏脚本号的处置口径）、栈下溢复用 `vm::FAULT_STACK`。
//!
//! **误用策略拍板（T3 worker 决定，记入报告）**：`move_enemy_to`/弹 setter族/`aim_player_angle`
//! 依赖"self 是某种 owner"的隐含契约（`move_enemy_to`/弹 setter族要求 owner 恰为对应池），
//! owner 类型不符 → **视为脚本作者违约，`Fault`**（响亮报错，非静默 no-op）——与
//! `docs/superpowers/sdd/task-3-brief.md` 建议一致："pick Fault unless codebase friction
//! suggests otherwise"，本仓一路复用既有 `FAULT_BAD_OP` 无额外摩擦，故采纳。`boss_set` 例外：
//! owner 非 ENEMY 时 `enemy` 字段写 `EnemyHandle::NULL`（不 Fault）——`boss_set` 语义本就
//! 允许 UI 槽先于 boss 敌人存在而调用（`world::boss_set` 文档："不校验 ui.enemy 句柄有效性"）。

use crate::boss::BossUiSlot;
use crate::bullets::{BulletHandle, BulletInit};
use crate::ecl::image::{SubId, SubKind};
use crate::ecl::shooter::{
    SH_ABS_OFFSET, SH_AIMED, SH_NO_TASK, SH_RING, SHOOTERS_PER_TASK, ShooterSlot,
};
use crate::ecl::task::{LOCALS, OWNER_BULLET, OWNER_ENEMY, Task};
use crate::ecl::vm::{FAULT_BAD_OP, FAULT_STACK, VmCtx};
use crate::enemy::{EnemyHandle, EnemyInit};
use crate::lasers::{ANCHOR_NONE, LaserHandle, LaserInit, LaserPool};
use crate::math::geom::polar_to_vec;
use crate::math::{Angle, Fx};
use crate::xform::XformSlot;

mod bullet;
mod enemy;
mod interp;
mod laser;
mod query;
mod shooter;
mod spell;
mod stage;

use bullet::*;
use enemy::*;
use interp::*;
use laser::*;
use query::*;
use shooter::*;
use spell::*;
use stage::*;

// ── syscall 号表（冻结；百分区制，2026-07-31 重排拍板）─────────────────────────
// 百位 = 族号：0xx `$` 引擎变量 / 1xx 查询 / 2xx 造物 / 3xx 弹操作 / 4xx 敌运动 /
// 5xx 局面·记账·道具 / 6xx shooter / 7xx 控制·事件·符卡 / 8xx+ 预留。
//
// **族内留空隙：新号落族内、永不乱序追加。** 族满了走评审开新族，不许溢出到隔壁——
// 上一版是十位族号制，`0x` 读族（容量 10）实占 13、`6x` shooter（容量 10）实占 15，
// 两次静默溢出之后 77 号起就没族可落了，最近四刀只好纯自增，读族因此裂成三段。
// 那次的教训不是"纪律松"，是**族容量对一张还在长的表本来就不够**。
//
// 4xx/5xx/6xx 与 ZUN ECL 的同号段**有意对齐**（他的 4xx = move、5xx = drops、
// 6xx = et* 弹管理器），方便对着 Priw8 的指令表读。
// 改号 = 冻结面变更 = 过评审 + bump ENGINE_VER（见 spec 2026-07-31）。
//
// **号域上限不受编码宽度约束**：`OP_SYS` 的操作数由发码侧 `SubBuilder::sys()` 写成
// `emit(no as u32)`（住**另一个 crate**：`stg-ecl-compiler/src/lib.rs`，不是本 crate 的
// `vm.rs`——`vm.rs` 只**读**这个操作数字）
// ——一个完整的 `u32` 字（`ecl::ops::ARITY[OP_SYS] == 1`，单位是"字"不是"字节"），不是
// 塞进 opcode 那个 `u8`。故 `u16` 全域（0..=65535）都可安全落进这个操作数，百分区制
// 一路扩到 `8xx`/`9xx` 甚至更高都不会撞编码位宽的墙——`3xx`–`7xx` 这些 ≥256 的号已经
// 是现成的证据。

// 0xx `$` 引擎变量（12；与 parse.rs::resolve_engine_var 白名单一一对应）／1xx 查询（10；
// 函数形态读口 + 纯数学）／2xx 造物（4）／3xx 弹操作（9；self owner 必须是 BULLET；按
// motion.rs 九连顺序编号）／4xx 敌运动（5；对齐 ZUN 4xx）／5xx 局面·记账·道具（14；
// 对齐 ZUN 5xx 的 drops；owner 类别无限制，STAGE 任务常发）／6xx shooter（15；对齐 ZUN
// 6xx 的 et*）／7xx 控制·事件·符卡·globals（7）／8xx 激光（10；激光池刀 Task 4）。
//
// 下方全部常量的**物理顺序沿用历史累加顺序（append-order）**，不随本刀按族重排位置——
// 每个常量的族由其**取值的百位**决定，不由物理位置决定；见文件头总纲。
pub const SYS_FRAME: u16 = 0;
pub const SYS_PLAYER_X: u16 = 10;
pub const SYS_PLAYER_Y: u16 = 11;
pub const SYS_SELF_X: u16 = 20;
pub const SYS_SELF_Y: u16 = 21;
pub const SYS_SELF_HP: u16 = 30;
pub const SYS_RAND_RANGE: u16 = 150;
/// 当前敌人的共享整数槽读口。
pub const SYS_SELF_INT: u16 = 170;
/// 当前敌人的共享定点槽读口。
pub const SYS_SELF_FX: u16 = 171;
/// 当前敌人的共享整数槽写口。
pub const SYS_SET_SELF_INT: u16 = 450;
/// 当前敌人的共享定点槽写口。
pub const SYS_SET_SELF_FX: u16 = 451;
pub const SYS_ACCUM_VALUE: u16 = 160;
pub const SYS_ACCEL_VALUE: u16 = 161;
pub const SYS_HERMITE_VALUE: u16 = 162;
pub const SYS_ACCUM_START: u16 = 750;
pub const SYS_ACCEL_START: u16 = 751;
pub const SYS_HERMITE_START: u16 = 752;

pub const SYS_GET_VAR: u16 = 700;
pub const SYS_SET_VAR: u16 = 701;
/// 任务龄（帧数，**M1.5 新增**）：`ctx.frame - task.born_frame`（wrapping）。**语义故意偏离
/// ZUN**（ZUN `-9988` 是"敌出生以来帧数"，只对敌有意义）——我们量的是**任务**的龄，不是
/// owner 实体的龄：零新状态（复用既有 `Task.born_frame`/`ctx.frame`），对全部 owner 种类
/// （含 STAGE）均有意义。见 `docs/ecl-ops.md`/`docs/zun-ecl-v2-reference.md` 的偏离记档。
pub const SYS_SELF_AGE: u16 = 32;
/// `$self_enemy`（boss 换段刀 spec §5.2）：owner 为 ENEMY → 打包敌号（同 [`SYS_SPAWN_ENEMY`] 返回值编码）；
/// 其余 owner → **-1**。与本族「非敌读 0」有意不同：打包敌号 0 合法。26 不是 op 号（不新增 F6 重叠点）。
pub const SYS_SELF_ENEMY: u16 = 26;
/// owner 上限血量（M1.5 新增）：owner=ENEMY → `enemies.hp_max[idx]`；非敌 → 押 0
/// （同 `SYS_SELF_HP` 误用策略：静默降级，不 Fault）。
pub const SYS_SELF_HP_MAX: u16 = 31;
/// 符卡计时读族（符卡机构 spec 2026-07-24 §5）：owner 绑定的 active 槽 → `frames_left`；
/// 无绑定 → `-1`（`wait_spell()` 语法糖的判据，同 `SYS_SELF_HP` 误用降级口径：owner
/// 非 ENEMY 直接押 -1，不 Fault）。
pub const SYS_SPELL_TIMER: u16 = 130;
/// `spell_result(slot)`（boss 换段刀 spec §3.3；owner 无限制）：押 `spell_last_result[slot]`
/// （0 还没结束过 / 1 血线 / 2 超时 / 3 手动）；`slot ∉ [0, MAX_BOSSES)` → 押 0 + 违约。
pub const SYS_SPELL_RESULT: u16 = 131;
/// 查敌读口(A5 补遗):活敌返 hp,其余 -1。1 参 `handle` = **打包敌号**(含 generation,
/// 见 [`crate::enemy::pack_handle`])。P4-b:越界/死槽/**gen 不符**/负值一律 -1,不 Fault
/// ——stage 编排等 boss 死的轮询原语。
///
/// **槽复用可辨(敌句柄打包刀 2026-07-31)**:敌死、槽被回收、另一只敌落进同一个槽之后,
/// 旧句柄读到的是 -1 而**不是**新那只敌的血。打包前这里是条真 bug——boss 死后若有杂兵
/// 占了它的槽,`enemy_hp(boss)` 会读到杂兵的血,"等 boss 死"的轮询就卡住不退。
pub const SYS_ENEMY_HP: u16 = 100;

// （原 "2x：写——创建/世界变更" 族——见文件头总纲：本区混有 2xx 造物 / 4xx 敌运动 /
// 7xx 控制·符卡三族，物理顺序沿用历史累加顺序，不代表族边界）
/// 丙方案 8 参（正序压栈）：`appearance, x, y, speed, angle, xform_off, xform_cnt, task_script`。
pub const SYS_CREATE_BULLET: u16 = 200;
/// 9 参：`appearance, x, y, n_angle, angle0, angle_step, n_speed, speed0, speed_step`（无 xform）。
pub const SYS_CREATE_BULLETS_BATCH: u16 = 201;
/// v1 直参 5 个：`x, y, hp, drop_table, score`（appearance 敌表后补，见 follow-ups）；
/// A5 乙案尾追 `sprite, task_script`。押**打包敌号**（[`crate::enemy::pack_handle`]：含
/// generation，恒非负）；池满 → **-1**。
///
/// **boss 换段刀（2026-09-14）调用约定变更**：压栈序追加实参与个数——
/// `x, y, hp, drop_table, score, sprite, task_script, arg0 … arg(n−1), argc`（无参时 `argc = 0`）。
/// 门禁全部先于建敌：argc 越 `[0, LOCALS]` 或栈不够 `argc + 7` → Fault(2)；task 为 none 却带参、
/// 或 task sub 不在册 / 非 Async / 形参个数 ≠ argc → Fault(0)。实参写进新任务 `locals[0..argc)`。
pub const SYS_SPAWN_ENEMY: u16 = 210;
/// 3 参：`x, y, item_type`。
pub const SYS_DROP_ITEM: u16 = 220;
/// self owner 敌；4 参：`dur, x, y, easing`。
pub const SYS_MOVE_ENEMY_TO: u16 = 400;
/// slot + 5 字段（`enemy` 取自 self owner，非显式参）：
/// `slot, hp_ratio, spell_id, timer_frames, phase_left, active`。
pub const SYS_BOSS_SET: u16 = 730;
/// 1 参：`ch`。
pub const SYS_PULSE_SIGNAL: u16 = 710;
/// 通道 B 渲染请求推送（M2 前置刀；D12/spec §2.5）。无 owner 类别限制——宣言/音效/震屏
/// 常由 STAGE 任务发。
pub const SYS_EMIT_REQ: u16 = 720;
/// 定位一次性演出（721；表现契约 v2；对应 ZUN `anmPlayPos`）：4 参 `x, y, kind, param`、
/// 无返回、owner 无限制。= `emit_req(REQ_FX_AT, [x raw, y raw, kind, param, 0, 0])` 的钉死
/// 布局版——`kind`/`param` 语义归内容包与壳侧约定，引擎不解释。即发即忘类。
pub const SYS_FX_AT: u16 = 721;
/// 依附一次性演出（722；对应 ZUN `anmPlay`）：2 参 `kind, param`、无返回、**self-only**
/// （owner 非 ENEMY → Fault(0)）。发 `REQ_FX_ATTACHED, [index, gen, kind, param, 0, 0]`，
/// 句柄取自 owner 敌；壳侧按 `(index, gen)` 每帧跟随、句柄失效即自毁。即发即忘类。
pub const SYS_FX_ON: u16 = 722;
/// 关卡结束（壳子刀 2026-09-07；表层 `stage_clear(stage)`）：1 参 `stage`，发**事件**
/// `EVT_STAGE_CLEARED{data0 = stage}`（通道 A 事实，不是通道 B 请求），owner 无限制。
/// 表层 codegen 在它后面追发 `PUSHI 1; WAIT`——本关到此为止，下一条语句在宿主放行后的
/// **第一帧**才执行；手写字节码只发 `SYS 723` 不会让出帧。
pub const SYS_STAGE_CLEAR: u16 = 723;
/// 符卡宣言（符卡机构 spec 2026-07-24 §5）：owner 必须 ENEMY（misuse → Fault）；7 参
/// 正序压栈 `slot, spell_id, pattern:SubRef, time_limit, bonus0, flags, hp_threshold`
/// （`pattern` 同 `fire` task 参同款 `SubRef`，负值=none）。
pub const SYS_SPELL_BEGIN: u16 = 740;
/// 符卡逃生舱口（符卡机构 spec 2026-07-24 §5）：无参；owner 绑定槽走 HP 路径结算，
/// 无绑定 → no-op（重复调用安全）。
pub const SYS_SPELL_END: u16 = 741;

// ── 8xx：激光（spec 2026-09-25-laser-pool-design §5；激光池刀 Task 4）─────────────
// 参数正序压栈、派发逆序弹出。`lz_*` 的 `lz` 是 `laser()` 返回的**打包激光句柄**——编码与
// 敌号逐条同款（`((generation & 0x7FFF) << 16) | index`，`-1` 是唯一无效哨兵；见
// [`pack_laser_handle`]）。代际只带低 15 位，重建句柄时一律取池里的完整 u16。
/// 建一条激光（9 参）：`laser(color, x, y, angle, len, width, warn, active, fade)`。
/// `color ∈ 0..=15` 否则 `FAULT_BAD_OP`（先验后建，同 `fire` 坏外观口径）；`warn/active/fade`
/// 出 `[0,65535]` 钳位并计一次 `contract_viol`；`start=0`、`end=start_len=len`、`speed=omega=0`；
/// 池满押 -1（P4-a，不 Fault）。返回打包句柄。
pub const SYS_LASER_CREATE: u16 = 800;
/// 形态二：设速率与近端长度（两字段世界层双边钳 `[0,LASER_LEN_MAX]`）。
pub const SYS_LASER_SPEED: u16 = 801;
/// 近端留空（原作第 4 关 `start = 64`）。
pub const SYS_LASER_START: u16 = 802;
/// 持续转动速率（BAM/帧，`i16`；栈值取低 16 位按位回绕为 i16，**不钳位、不计数**）。
pub const SYS_LASER_OMEGA: u16 = 803;
/// 一次性转一个角度（回绕加）。
pub const SYS_LASER_ROTATE: u16 = 804;
/// 指向自机 0 的角度 + 偏移。
pub const SYS_LASER_AIM: u16 = 805;
/// 挂到敌人身上；`enemy = -1` 传 `EnemyHandle::NULL` 表示解除；其它无法解析的敌号视同失效
/// 句柄（不挂靠 + 计一次违约）。
pub const SYS_LASER_ANCHOR: u16 = 806;
/// 直接设置原点（同时解除挂靠）。
pub const SYS_LASER_ORIGIN: u16 = 807;
/// 取消：`state < 2 → 2`、`timer = 0`。
pub const SYS_LASER_CANCEL: u16 = 808;
/// 是否存活（只读，不计数），押 0/1。
pub const SYS_LASER_ALIVE: u16 = 809;
/// 几何读口（810–814，只读、不计数；失效句柄押 0——**0 不是哨兵**，先 `lz_alive` 探活）：
/// 原点 x / 原点 y / 角度（BAM）/ 近端偏移 `start` / 远端偏移 `end`。读的是调用时刻池里的值：
/// 相位 2 里读到的是上一帧相位 5 推进后的结果，加上本帧此前 ECL 写口做的修改。
pub const SYS_LASER_X: u16 = 810;
pub const SYS_LASER_Y: u16 = 811;
pub const SYS_LASER_ANGLE: u16 = 812;
pub const SYS_LASER_NEAR: u16 = 813;
pub const SYS_LASER_FAR: u16 = 814;

// 3xx：弹操作族（self owner 必须是 BULLET；按 motion.rs 九连顺序编号）
pub const SYS_SET_BULLET_SPEED: u16 = 300;
pub const SYS_SET_BULLET_ANGLE: u16 = 301;
pub const SYS_TURN_BULLET: u16 = 302;
pub const SYS_SET_BULLET_VEL: u16 = 310;
pub const SYS_SET_BULLET_ANG_VEL: u16 = 311;
pub const SYS_SET_BULLET_ACCEL: u16 = 312;
pub const SYS_SET_BULLET_GRAVITY: u16 = 313;
pub const SYS_STOP_BULLET_FX: u16 = 320;
pub const SYS_AIM_BULLET_AT_PLAYER: u16 = 330;

// （原 "4x：读——瞄准" 族——本条单独在新表里落 1xx 查询族，见文件头总纲）
/// 0 参：读 self 位置 → 朝向 P0 的角度（BAM，供脚本自算瞄准环）。
pub const SYS_AIM_PLAYER_ANGLE: u16 = 120;

// 5xx：局面·记账·道具族（整局流程刀 spec §4；owner 类别无限制，STAGE 任务常发）。
/// 1 参：`delta`（允许负，饱和钳 `[0, u64::MAX]`，P4-b）。
pub const SYS_ADD_SCORE: u16 = 500;
/// 1 参：`id`（`0..=65535` 收窄，越界 no-op+viol）。写 `bgm_id` + 发 `REQ_BGM`。
pub const SYS_BGM: u16 = 550;
/// 同上，写 `bg_id` + `REQ_BG`。
pub const SYS_BG: u16 = 551;
/// 1 参：`n`。写 `bg_phase` + 自动盖 `bg_phase_frame` = 当前帧 + 发 `REQ_BG_PHASE`。
pub const SYS_BG_PHASE: u16 = 552;
/// 停住**自机**的时间（自机能力刀，ECL 演出方向）：写 `freeze_left[1]` ⇒ 冻 A+B
/// （自机不能移动/发新弹，已在场上的自机弹也冻住），敌方照常行动。
/// `frames = 0` 即**立即解除**；重入取覆盖（后写为准）。
pub const SYS_TIME_STOP_PLAYER: u16 = 560;
/// 1 参：`r`（`Fx` raw）。改写自机 0 的**中弹判定半径**（碰撞行 1 / 3 的被动操作数；擦弹半径不动）。
/// 出场值仍由角色表决定（数据驱动），本 syscall 是局中的覆写口——演出（放大 / 缩小判定）与外部驱动同一条路
/// （`WorldBody::set_player_hit_radius`）。钳 `[0, MAX_ENTITY_RADIUS]`、钳了计 `contract_viol`（同 441 `set_hitbox`）；
/// 持续到下次改写或开新局（Classic 死亡重生不重置）。
pub const SYS_SET_PLAYER_HITBOX: u16 = 561;
/// 全场清弹（B19；0 参、无返回）。铺一个覆盖全场、`life=1` 的 `FIELD_CLEAR_BULLETS`
/// 作用区——**复用现成的消弹区机制**，故"每颗被消的弹原位转一颗星星"（M0-15）与
/// `EVT_FIELD_CLEARED` 都是白送的，引擎侧零新机制（同 `settle_one_spell` 的全屏清弹样板）。
///
/// 关底转场（`REQ_STAGE_CLEAR` 挂牌前）是首个真实消费者。**不给护盾帧**——那是 bomb
/// 那刀的职责（bomb = `FIELD_CLEAR_BULLETS | FIELD_DAMAGE` + 自机无敌）。
/// P4-a：field 池满 → `create_field` 自身的降级（NULL + 计数），本 syscall 不 Fault。
pub const SYS_CLEAR_BULLETS: u16 = 540;
/// `clear_bullets_at(x, y, r, stars)`（boss 换段刀 spec §6；owner 无限制）：圆形一帧清弹区，
/// `stars == 0` 带 `FIELD_NO_STAR`（清而不转星）。半径钳制走 `create_field`。
pub const SYS_CLEAR_BULLETS_AT: u16 = 541;
/// 残机增量（B20；1 参 `delta`、无返回）。双边钳 `[0, u8::MAX]`（P4-b：`delta` 是脚本给的
/// 任意 `i32`，先 `saturating_add` 再 `clamp`，不回绕不 panic）。
///
/// **增量形态（`add_*`）是人类裁定**，不是漏了 `set_*`：绝对赋值的唯一确定场景（开局装备）
/// 已被 [`crate::player::Loadout`]（`World::new_game_at` 的装备参）收编，运行中脚本要的
/// 都是"奖命 +1 / 中弹 −1"这类记账。
/// 别把这族"补全"成 `set_lives`/`set_bombs`/`set_power` 四件套——多一条写路径就多一处
/// 与 `Loadout` 抢开局初值的歧义。
pub const SYS_ADD_LIVES: u16 = 510;
/// X 键库存增量（Chronos 停止 / Classic bomb 共用，钳 `[0, STOP_STOCK_MAX]`；B20）。语义同 [`SYS_ADD_LIVES`]；增量形态同为人类裁定。
pub const SYS_ADD_BOMBS: u16 = 511;
/// 火力增量（B20）。语义同 [`SYS_ADD_LIVES`]，但上钳是 [`crate::items::POWER_MAX`]（400，
/// = 显示 4.00）**而非 `u16::MAX`**——越过它 `power_tier` 索引就 OOB（见
/// `world::WorldBody::set_player_power` 文档）。增量形态同为人类裁定。
pub const SYS_ADD_POWER: u16 = 512;
// 513 退役（玩法刀 2026-09-14，原 add_time_stops）：号不复用。
/// 清空自身待掉落计数（520；0 参、无返回。敌人死亡效果刀，参照 ZUN ECL 的 `dropClear` 506）。
/// self owner 必须是 ENEMY，否则 Fault（misuse 策略，同 `move_enemy_to`）。
pub const SYS_DROP_CLEAR: u16 = 520;
/// 给自身待掉落计数增量加 `n` 颗 `type`（521；2 参、无返回。参照 ZUN `dropExtra` 507）。
/// **只增不减**是人类裁定——要清空用 `drop_clear()`。坏类型/负 n 的处置见
/// [`crate::world::WorldBody::add_enemy_drop`]。
pub const SYS_DROP_ADD: u16 = 521;
/// 立刻把自身待掉落计数撒出去（522；0 参、无返回。参照 ZUN `dropItems` 509）。
/// **吐完不清空**（人类裁定 D-3，照 ZUN 字面）——故 `drop_items(); die();` 掉**双份**，
/// 作者自负。这条语义有测试钉死（`drop_items_does_not_clear_counts_...`），别"顺手修好"。
pub const SYS_DROP_ITEMS: u16 = 522;
/// 就地阵亡：跑完整死亡效果（530；0 参、无返回。参照 ZUN `die` 561）。
/// **表层 `die()` 降低成本 syscall + `OP_KILL_SELF` 两条指令**（见 codegen），故调用它的
/// 任务立即终止（人类裁定 D-4）。ZUN 的 561 还经 `setDeath`(556) 间接一层——那半留给
/// `death_script` 通电那一刀，届时与 ZUN 完全同构。
pub const SYS_DIE: u16 = 530;
/// `kill_all_enemies(mode)`（boss 换段刀 spec §5.3；owner 无限制）：升序杀除调用者/免清/已死之外的敌；
/// `KILL_SILENT(0)` 静默、`KILL_DIE(1)` 同 `die()`，其它 mode → no-op + 违约。
pub const SYS_KILL_ALL_ENEMIES: u16 = 531;

// ── Shooter：预存发射参数集（600-660；shooter 刀 2026-07-31，参照 ZUN et* 族 600-641）──
//
// 每任务 4 个编号槽（[`crate::ecl::shooter::SHOOTERS_PER_TASK`]）。`id ≥ SHOOTERS_PER_TASK`
// 一律 no-op + `contract_viol`（P4-b）——不 Fault，因为"槽号写错"是常见笔误而非结构性违约，
// 降级比杀任务更有用。判据集中在 [`shooter_mut`] 一处，15 个派发臂共用（`sh_fire` 同口径）。
//
// 下面**前 14 条**都只是写字段、无副作用：不查 appearance 是否在册、不查 xform 区间、不查
// sub 号在册——那些校验统一在**开火那一刻**做（同 `fire` 的"先验后建"口径：设参数时弹还
// 不存在，没有可拒绝的对象）。`sh_fire`(660) 的语义见其自身文档。
pub const SYS_SH_RESET: u16 = 600;
pub const SYS_SH_SPRITE: u16 = 610;
pub const SYS_SH_OFFSET: u16 = 620;
pub const SYS_SH_OFFSET_ABS: u16 = 621;
pub const SYS_SH_OFFSET_RAD: u16 = 622;
pub const SYS_SH_DIST: u16 = 623;
pub const SYS_SH_ANGLE: u16 = 630;
pub const SYS_SH_SPEED: u16 = 631;
pub const SYS_SH_COUNT: u16 = 632;
pub const SYS_SH_AIM: u16 = 640;
pub const SYS_SH_RING: u16 = 641;
pub const SYS_SH_XFORM: u16 = 650;
pub const SYS_SH_TASK: u16 = 651;
pub const SYS_SH_REQ: u16 = 652;
/// 开火（660）：用发射器槽 `id` 的参数造弹。**无返回值**（人类裁定 D-8——本语言要求值必须
/// 消费，有返回值就得写 `_ = sh_fire(0);`，而开火是循环里最高频的语句）。
///
/// 七步见 spec §10。要点三条，都有判别式测试钉着，**别"顺手改好"**：
/// - **fan 以基准方向为中心对称展开**（D-7）；ring 不居中。
/// - `dist` 逐颗沿**各自**角度位移，不是整环平移。
/// - 直角偏移与极坐标偏移**相加**（ZUN 626 明写 stacks），不是覆盖。
pub const SYS_SH_FIRE: u16 = 660;

// ── 数学/查询面（110/140/141；小清洗刀 2026-07-31）────────────────────────────────
/// 反正切（140）：2 参 `y, x`（**都是 `Fx` raw**），押 BAM 角。核里的
/// [`crate::math::cordic::atan2`]（整数 CORDIC，钉死 16 轮）此前脚本够不着——`aim_player`
/// 只能瞄自机，这条能瞄任意点/任意敌。
///
/// **无 P4 分支**：CORDIC 对任意 `(y, x)` 都有定义，含 `(0, 0)`（返 `Angle::ZERO`）——
/// 没有"坏参数"这个概念，故不计 `contract_viol`、不 Fault。
pub const SYS_ATAN2: u16 = 140;
/// 向量模（141）：2 参 `dx, dy`（`Fx` raw），押 `Fx` raw。**不是两点距离**——两点距离由
/// 脚本自己减（`dist(bx - ax, by - ay)`）。
///
/// 实现 = `isqrt(len_sq(dx, dy))`：[`crate::math::geom::len_sq`] 返的是 **Q32.32**
/// （`raw²`，不归一化，见 CLAUDE.md"定点乘法规范"），而 [`crate::math::isqrt::isqrt`]
/// 开根**正好把 Q32.32 变回 Q16.16** —— 两个原语都在核里，此前脚本一个都够不着。
///
/// **为什么不单独暴露 `len_sq`/`isqrt`**（人类裁定）：`len_sq` 返 i64 而脚本值域是 i32，
/// 装不下；单独的 `isqrt` 对脚本没有直接用处。`dist` 才是那个有用的组合。
///
/// 值域：`len_sq` 恒 ≥ 0 故 `as u64` 安全。收窄回 i32 分两个域看——
/// - **世界坐标差**（现实用法）：满屏最大约 1.7e15 → `isqrt` ≈ 4.1e7，远在 i32 上限
///   （2.1e9）之内，余量两个数量级；
/// - **任意 `fx` 输入**（本条是通用两参 syscall，脚本能直接喂极端值）：`dx = dy = i32::MAX`
///   时 `len_sq` ≈ 9.22e18（贴着 i64 上限但不溢出），`isqrt` = 3037000498 > `i32::MAX`
///   ⇒ 裸 `as i32` 会**静默回绕成负数**（-1257966798）。故收窄处**饱和**
///   （`.min(i32::MAX as u32)`）：把"负距离"这个会往下游传播的无意义值换成"确定性降级到
///   最大可表示距离"，与 P4 一贯取向同侧。钉在 `dist_saturates_instead_of_wrapping_negative`。
///
/// 无 P4 计数分支：饱和是正常语义（同 `add_score`/`add_lives` 族的钳位口径），
/// 不计 `contract_viol`、不 Fault。
pub const SYS_DIST: u16 = 141;
/// 最近敌查询（110）：2 参 `x, y`（`Fx` raw），押**打包敌号**；场上无敌（或全 dying）→ **-1**。
///
/// [`crate::world::WorldBody::nearest_enemy`] 自 M0-13 建完就是死代码（有实现、有测试、
/// 从没有 syscall 暴露过），本条只是给它通电，世界侧一行未改。
///
/// 编码同 [`SYS_ENEMY_HP`]（100）/[`SYS_SPAWN_ENEMY`]（210）——三条本就是配对使用的
/// （拿号 → 轮询）。世界侧返的一直是**完整句柄**，敌句柄打包刀（2026-07-31）之前
/// 这里只押 `index`、把 generation 丢了；接上之后槽复用可辨。
/// owner 类别无限制（关卡任务也该能查）。
pub const SYS_NEAREST_ENEMY: u16 = 110;

// ── 敌坐标读口（101-102；敌坐标读口刀 2026-07-31）──────────────────────────────
/// 按敌号读 **x**（101）：1 参 `handle`（**打包敌号**，与 [`SYS_ENEMY_HP`]/
/// [`SYS_NEAREST_ENEMY`] 同口径——含 generation，槽复用可辨），押 `Fx` raw。
///
/// 补的是上一刀（110/140/141 通电）暴露的断头路：脚本拿得到敌号却读不到坐标，
/// "查最近的敌 → 朝它开火"算不出角度。数据本就在敌池里躺着，缺的只是读口。
///
/// P4-b 降级（照 `sys_enemy_hp`）：**负句柄 / 越界 / 死槽 / gen 不符 → 返 `0`**，不 Fault、
/// **不计 `contract_viol`**（纯读族口径）；`ENEMY_DYING` 的敌**仍可读**（`is_alive` 是
/// 存活位，dying 只是 flag，槽活到相位 9 才回收）；owner 类别**无限制**（关卡任务也该能查）。
///
/// **为什么降级值是 `0` 而不是哨兵**：`enemy_hp` 能用 `-1` 是因为 hp 天然非负；坐标没有
/// 这个便利——`-1` 是完全合法的 `Fx` raw（≈ −0.0000153），**没有哨兵位可用**。取 `Fx::ZERO`
/// 与 `self_pos` 对 STAGE owner 的第三分支同款。代价是"敌恰在原点"与"号无效"读起来一样，
/// 故探活归脚本：先 `enemy_hp(e) != -1` 再读坐标（手册 `docs/ecl-lang.md` 写死了这条惯例）。
/// ⚠️ 探活判据**不是** `enemy_hp(e) >= 0`——overkill 的敌 hp 是真实负值
/// （`settle::kill_enemy` 只 `min(0)`，不抹平负血），`>= 0` 会把"刚被打穿、槽还在"的敌误判成无效。
pub const SYS_ENEMY_X: u16 = 101;
/// 按敌号读 **y**（102）——镜像 [`SYS_ENEMY_X`]，语义/降级/owner 口径逐条相同。
pub const SYS_ENEMY_Y: u16 = 102;

// ── 探活读口（103；探活读口刀 2026-07-31）────────────────────────────────────
/// 敌**探活**（103，ZUN `555 enmAlive`）：1 参 `handle`（**打包敌号**，同读族口径），
/// 押 **1 或 0**。
///
/// 补的是上一刀留下的残余缝：探活此前只能拿 `enemy_hp(e) != -1` 当探针，而 `-1`
/// **同时是降级值和一个合法血量**——overkill 的敌 hp 是真实负值（`settle::kill_enemy`
/// 只 `min(0)`，不抹平），血量恰为 −1 的活敌会被旧探针误判成"号无效"。本号是专用口，
/// 与血量取值无关。
///
/// **判据逐字同 [`SYS_ENEMY_HP`]/`sys_enemy_pos`**（同一个 [`resolve_enemy_handle`]）：
/// `packed >= 0 && idx < CAP && is_alive(idx) && (generation[idx] & 0x7FFF) == g`。
/// 不 Fault、**不计 `contract_viol`**（纯读族口径）；owner 类别**无限制**。
///
/// **语义裁定（人类拍板）：判的是「槽有效」，含 `ENEMY_DYING` 的敌 → 返 1，不是「还能打」。**
/// 理由是读族四条（`enemy_hp`/`enemy_x`/`enemy_y`/`enemy_alive`）必须用**完全相同**的三判据
/// ——dying 的槽要活到相位 9（坐标仍读得到），四条里单独给一条换判据会让这组口径散掉。
/// 配套建议写在手册：[`crate::world::WorldBody::nearest_enemy`] **本身已排除 dying**
/// （候选 = 存活且非 dying），所以"从它拿到的号后来变 dying"应当**重查**而不是继续用。
pub const SYS_ENEMY_ALIVE: u16 = 103;

// ── 敌人运动动词族（410-421；T4，对标 ZUN ECL `move` 400-447 族）────────────────
// 四条 syscall 都是 `world/motion.rs` 四个 `set_enemy_*` 写 API 的薄封装：owner 违约
// （非 ENEMY）→ Fault，悬垂句柄/无效缓动编号的 P4-b 校验全在世界层做过，本层不重复计数。

/// 极坐标速度（410；ZUN `404 moveVel` / `405 moveVelTime`）：4 参正序
/// `dur, angle, speed, easing`。self owner 非 ENEMY → Fault（同 [`SYS_MOVE_ENEMY_TO`]）。
/// `dur == 0` 是合法退化 = 立即设。
pub const SYS_MOVE_VEL: u16 = 410;
/// 笛卡尔速度（411）：4 参正序 `dur, vx, vy, easing`。**`dur > 0` 时在笛卡尔空间插值**
/// ——不转极坐标，否则它就退化成 [`SYS_MOVE_VEL`] 的语法糖（spec §3.3）。
pub const SYS_MOVE_VEL_XY: u16 = 411;
/// 只转向、保持速率（420；ZUN `440 moveAngle`）：3 参正序 `dur, angle, easing`。
pub const SYS_MOVE_ANGLE: u16 = 420;
/// 只调速、保持方向（421；ZUN `444 moveSpeed`）：3 参正序 `dur, speed, easing`。
pub const SYS_MOVE_SPEED: u16 = 421;
/// 敌人表现状态号（430；表现契约 v2，2026-09-07；对应 ZUN `anmSelect`/`anmSetMain`/
/// `anmInterrupt` 那族的**电平版**）。1 参 `state`、无返回；**self-only**（owner 非 ENEMY →
/// Fault(0)，同 400 族）。写 `anm_state = state as u16` 并**无条件**盖 `anm_state_frame =
/// 当前帧——同状态重设 = 重播。世界不解释状态号；表现层按 `(sprite, anm_state, state_age)`
/// 选帧（`render-contract.md` §7）。`state` 截 u16（负数/超界折叠，同 sprite 号口径）。
pub const SYS_SET_ANM_STATE: u16 = 430;
/// 敌判定族 44x（boss 换段刀 spec §5.1；self-only，owner 非 ENEMY → Fault(0)）：
/// `set_invuln(frames)`——覆写 `invuln`，`frames ∉ [0,65535]` → no-op + 违约。
pub const SYS_SET_INVULN: u16 = 440;
/// `set_hitbox(r)`——体碰半径 `radius`（碰撞行 3），钳 `[0, MAX_ENTITY_RADIUS]` + 违约。
pub const SYS_SET_HITBOX: u16 = 441;
/// `set_hurtbox(r)`——受击半径 `hurtbox`（碰撞行 4/7），同上钳制。
pub const SYS_SET_HURTBOX: u16 = 442;
/// `set_enemy_flag(flag, on)`——`flag` 须为 `ENEMY_NO_BODY | ENEMY_KILLALL_EXEMPT` 的非空子集，否则 no-op + 违约。
pub const SYS_SET_ENEMY_FLAG: u16 = 443;

// （原 "── $self_* 速度引擎变量（87-90）──" 族——本四条新表里落 0xx `$` 引擎变量族，
// 见文件头总纲；T5；白名单 8→12）
// 与 `$self_x`/`$self_y` 同族同形状：不收参数，owner 从 task 取，派发规则逐条同
// `self_pos`（ENEMY→敌池/BULLET→弹池/其余→0）。存在的理由：笛卡尔没有单轴动词
// （`move_vel_xy` 两轴齐写），"只插 vy、保住 vx"唯一的写法就是把当前 vx 读出来填回去。

/// owner 的笛卡尔速度 x（022）：ENEMY → 敌池、BULLET → 弹池、其余 → 0（同 [`SYS_SELF_X`]）。
/// 存在的理由：笛卡尔没有单轴动词，"只插 vy 保住 vx"唯一的写法是把当前 vx 读出来填回去。
pub const SYS_SELF_VX: u16 = 22;
/// owner 的笛卡尔速度 y（023）——镜像 [`SYS_SELF_VX`]。
pub const SYS_SELF_VY: u16 = 23;
/// owner 的速率（024，作者视图）：与 [`SYS_SELF_VX`]/[`SYS_SELF_VY`] 恒同步（双表示）。
pub const SYS_SELF_SPEED: u16 = 24;
/// owner 的朝向（025，作者视图，BAM）。近停时冻结（`BACKFILL_MIN_SPEED`），故零速下
/// 读到的是**最后一次有效朝向**而非垃圾角。
pub const SYS_SELF_ANGLE: u16 = 25;

/// 号表白名单：`no` 是不是一条 [`dispatch`] 真的会派发的 syscall——**按表查而非比大小**
/// （百分区制下号非连续，同 [`crate::ecl::ops::op_implemented`] 的纪律）。不在表内的号
/// 由 `dispatch` 的兜底臂返 `FAULT_BAD_OP`。
///
/// **存在的理由是跨 crate**（`dispatch` 是 `pub(crate)`、冻结号表是 `cfg(test)`，
/// 编译器 crate 两个都够不着）：`stg-ecl-compiler` 的 `builtins::BUILTINS` 要能断言
/// "`is_op == false` 的条目，其 `syscall` 字段装的确实是个会被派发的号"。见
/// `builtins.rs::builtin_dispatch_kind_matches_what_the_field_holds`。
///
/// **与 `dispatch` 的同步靠测试押运，不靠自律**：`syscall_whitelist_matches_the_frozen_table`
/// 断言"全 `u16` 域里为真的号恰好是冻结表中的113条"，漏一条/多一条即红。
pub const fn syscall_implemented(no: u16) -> bool {
    matches!(
        no,
        // 0xx `$` 引擎变量
        SYS_FRAME
            | SYS_PLAYER_X
            | SYS_PLAYER_Y
            | SYS_SELF_X
            | SYS_SELF_Y
            | SYS_SELF_VX
            | SYS_SELF_VY
            | SYS_SELF_SPEED
            | SYS_SELF_ANGLE
            | SYS_SELF_HP
            | SYS_SELF_HP_MAX
            | SYS_SELF_AGE
            | SYS_SELF_ENEMY
            // 1xx 查询
            | SYS_ENEMY_HP
            | SYS_ENEMY_X
            | SYS_ENEMY_Y
            | SYS_ENEMY_ALIVE
            | SYS_NEAREST_ENEMY
            | SYS_AIM_PLAYER_ANGLE
            | SYS_SPELL_TIMER
            | SYS_SPELL_RESULT
            | SYS_ATAN2
            | SYS_DIST
            | SYS_RAND_RANGE
            | SYS_SELF_INT
            | SYS_SELF_FX
            // 2xx 造物
            | SYS_CREATE_BULLET
            | SYS_CREATE_BULLETS_BATCH
            | SYS_SPAWN_ENEMY
            | SYS_DROP_ITEM
            // 3xx 弹操作
            | SYS_SET_BULLET_SPEED
            | SYS_SET_BULLET_ANGLE
            | SYS_TURN_BULLET
            | SYS_SET_BULLET_VEL
            | SYS_SET_BULLET_ANG_VEL
            | SYS_SET_BULLET_ACCEL
            | SYS_SET_BULLET_GRAVITY
            | SYS_STOP_BULLET_FX
            | SYS_AIM_BULLET_AT_PLAYER
            // 4xx 敌运动
            | SYS_MOVE_ENEMY_TO
            | SYS_MOVE_VEL
            | SYS_MOVE_VEL_XY
            | SYS_MOVE_ANGLE
            | SYS_MOVE_SPEED
            | SYS_SET_ANM_STATE
            | SYS_SET_INVULN
            | SYS_SET_HITBOX
            | SYS_SET_HURTBOX
            | SYS_SET_ENEMY_FLAG
            | SYS_SET_SELF_INT
            | SYS_SET_SELF_FX
            // 5xx 局面·记账·道具
            | SYS_ADD_SCORE
            | SYS_ADD_LIVES
            | SYS_ADD_BOMBS
            | SYS_ADD_POWER
            | SYS_DROP_CLEAR
            | SYS_DROP_ADD
            | SYS_DROP_ITEMS
            | SYS_DIE
            | SYS_KILL_ALL_ENEMIES
            | SYS_CLEAR_BULLETS
            | SYS_CLEAR_BULLETS_AT
            | SYS_BGM
            | SYS_BG
            | SYS_BG_PHASE
            | SYS_TIME_STOP_PLAYER
            | SYS_SET_PLAYER_HITBOX
            // 6xx shooter
            | SYS_SH_RESET
            | SYS_SH_SPRITE
            | SYS_SH_OFFSET
            | SYS_SH_OFFSET_ABS
            | SYS_SH_OFFSET_RAD
            | SYS_SH_DIST
            | SYS_SH_ANGLE
            | SYS_SH_SPEED
            | SYS_SH_COUNT
            | SYS_SH_AIM
            | SYS_SH_RING
            | SYS_SH_XFORM
            | SYS_SH_TASK
            | SYS_SH_REQ
            | SYS_SH_FIRE
            // 7xx 控制·事件·符卡·globals
            | SYS_ACCUM_VALUE | SYS_ACCEL_VALUE | SYS_HERMITE_VALUE
            | SYS_ACCUM_START | SYS_ACCEL_START | SYS_HERMITE_START
            | SYS_GET_VAR
            | SYS_SET_VAR
            | SYS_PULSE_SIGNAL
            | SYS_EMIT_REQ
            | SYS_FX_AT
            | SYS_FX_ON
            | SYS_STAGE_CLEAR
            | SYS_BOSS_SET
            | SYS_SPELL_BEGIN
            | SYS_SPELL_END
            // 8xx 激光
            | SYS_LASER_CREATE
            | SYS_LASER_SPEED
            | SYS_LASER_START
            | SYS_LASER_OMEGA
            | SYS_LASER_ROTATE
            | SYS_LASER_AIM
            | SYS_LASER_ANCHOR
            | SYS_LASER_ORIGIN
            | SYS_LASER_CANCEL
            | SYS_LASER_ALIVE
            | SYS_LASER_X
            | SYS_LASER_Y
            | SYS_LASER_ANGLE
            | SYS_LASER_NEAR
            | SYS_LASER_FAR
    )
}

// ── 求值栈存取（供各 syscall 实现复用；语义同 vm::exec 内的 pop!/push! 宏）───────

fn pop(task: &mut Task) -> Result<i32, u8> {
    if task.sp == 0 {
        return Err(FAULT_STACK);
    }
    task.sp -= 1;
    Ok(task.stack[task.sp as usize])
}

fn push(task: &mut Task, v: i32) -> Result<(), u8> {
    if task.sp as usize >= crate::ecl::task::EVAL_DEPTH {
        return Err(FAULT_STACK);
    }
    task.stack[task.sp as usize] = v;
    task.sp += 1;
    Ok(())
}

/// self 位置（owner 已被调度层 gate 校验存活——见 `vm::run_tasks` 文档）：
/// ENEMY→敌池位置 / BULLET→弹池位置 / STAGE→(0,0)。
fn self_pos(task: &Task, ctx: &VmCtx) -> (Fx, Fx) {
    match task.owner_kind {
        OWNER_ENEMY => {
            let i = task.owner_index as usize;
            (ctx.body.enemies.x[i], ctx.body.enemies.y[i])
        }
        OWNER_BULLET => {
            let i = task.owner_index as usize;
            (ctx.body.bullets.x[i], ctx.body.bullets.y[i])
        }
        _ => (Fx::ZERO, Fx::ZERO),
    }
}

/// owner 的速度四件（`$self_vx`/`$self_vy`/`$self_speed`/`$self_angle` 共用）。
/// 派发规则逐条同 [`self_pos`]：ENEMY → 敌池、BULLET → 弹池、其余 → 全零。
/// 返回 `(vx, vy, speed, angle_raw)`，四个都已是可直接押栈的 raw。
///
/// BULLET 分支需要 `&mut`：CART_FX 惰性化后 `speed`/`angle` 可能陈值（引擎第二刀 §5），
/// 读之前先 `materialize_polar`（读取顺带回写，读完 `BULLET_POLAR_STALE` 清零）。
fn self_vel(task: &Task, ctx: &mut VmCtx) -> (i32, i32, i32, i32) {
    match task.owner_kind {
        OWNER_ENEMY => {
            let i = task.owner_index as usize;
            (
                ctx.body.enemies.vx[i].raw(),
                ctx.body.enemies.vy[i].raw(),
                ctx.body.enemies.speed[i].raw(),
                ctx.body.enemies.angle[i].raw() as i32,
            )
        }
        OWNER_BULLET => {
            let i = task.owner_index as usize;
            ctx.body.materialize_polar(i);
            (
                ctx.body.bullets.vx[i].raw(),
                ctx.body.bullets.vy[i].raw(),
                ctx.body.bullets.speed[i].raw(),
                ctx.body.bullets.angle[i].raw() as i32,
            )
        }
        _ => (0, 0, 0, 0),
    }
}

fn self_hp(task: &Task, ctx: &VmCtx) -> i32 {
    match task.owner_kind {
        OWNER_ENEMY => ctx.body.enemies.hp[task.owner_index as usize],
        _ => 0,
    }
}

/// owner 上限血量：ENEMY→`hp_max`／非敌恒 0（`SYS_SELF_HP_MAX`，M1.5；镜像 `self_hp`）。
fn self_hp_max(task: &Task, ctx: &VmCtx) -> i32 {
    match task.owner_kind {
        OWNER_ENEMY => ctx.body.enemies.hp_max[task.owner_index as usize],
        _ => 0,
    }
}

/// self owner 必须是 BULLET，否则脚本作者违约 → `Fault`（misuse 策略，见模块文档）。
fn self_bullet_handle(task: &Task) -> Result<BulletHandle, u8> {
    if task.owner_kind != OWNER_BULLET {
        return Err(FAULT_BAD_OP);
    }
    Ok(BulletHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    })
}

/// self owner 必须是 ENEMY，否则脚本作者违约 → `Fault`（misuse 策略，见模块文档）。
fn self_enemy_handle(task: &Task) -> Result<EnemyHandle, u8> {
    if task.owner_kind != OWNER_ENEMY {
        return Err(FAULT_BAD_OP);
    }
    Ok(EnemyHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    })
}

/// 号表派发。坏号/参数不足 → `Err(fault_code)`（由 `vm::exec` 转 `Exec::Fault`）。
pub(crate) fn dispatch(no: u16, task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    match no {
        // ── 0xx `$` 引擎变量 ──────────────────────────────────────────────────
        SYS_FRAME => push(task, ctx.frame as i32),
        SYS_PLAYER_X => push(task, ctx.body.players[0].x.raw()),
        SYS_PLAYER_Y => push(task, ctx.body.players[0].y.raw()),
        SYS_SELF_ENEMY => sys_self_enemy(task, ctx),
        SYS_SELF_X => sys_self_x(task, ctx),
        SYS_SELF_Y => sys_self_y(task, ctx),
        // ── $self_* 速度引擎变量 022-025（T5）──────────────────────────────────
        SYS_SELF_VX => sys_self_vx(task, ctx),
        SYS_SELF_VY => sys_self_vy(task, ctx),
        SYS_SELF_SPEED => sys_self_speed(task, ctx),
        SYS_SELF_ANGLE => sys_self_angle(task, ctx),
        SYS_SELF_HP => sys_self_hp(task, ctx),
        SYS_SELF_HP_MAX => sys_self_hp_max(task, ctx),
        SYS_SELF_AGE => sys_self_age(task, ctx),

        // ── 1xx 查询 ─────────────────────────────────────────────────────────
        SYS_ENEMY_HP => sys_enemy_hp(task, ctx),
        // ── 敌坐标读口 101/102（敌坐标读口刀）──────────────────────────────────
        SYS_ENEMY_X => sys_enemy_pos(task, ctx, false),
        SYS_ENEMY_Y => sys_enemy_pos(task, ctx, true),
        // ── 探活读口 103（探活读口刀）────────────────────────────────────────
        SYS_ENEMY_ALIVE => sys_enemy_alive(task, ctx),
        SYS_NEAREST_ENEMY => sys_nearest_enemy(task, ctx),
        SYS_AIM_PLAYER_ANGLE => sys_aim_player_angle(task, ctx),
        SYS_SPELL_TIMER => sys_spell_timer(task, ctx),
        SYS_SPELL_RESULT => sys_spell_result(task, ctx),
        // ── 数学/查询面 110/140/141（参数**逆序弹出**，照 `sys_move_enemy_to`）────────────
        SYS_ATAN2 => {
            let x = pop(task)?;
            let y = pop(task)?;
            let a = crate::math::cordic::atan2(Fx::from_raw(y), Fx::from_raw(x));
            push(task, a.raw() as i32)
        }
        SYS_DIST => sys_dist(task, ctx),
        SYS_RAND_RANGE => sys_rand_range(task, ctx),
        SYS_SELF_INT => sys_self_int(task, ctx),
        SYS_SELF_FX => sys_self_fx(task, ctx),

        // ── 2xx 造物 ─────────────────────────────────────────────────────────
        SYS_CREATE_BULLET => sys_create_bullet(task, ctx),
        SYS_CREATE_BULLETS_BATCH => sys_create_bullets_batch(task, ctx),
        SYS_SPAWN_ENEMY => sys_spawn_enemy(task, ctx),
        SYS_DROP_ITEM => sys_drop_item(task, ctx),

        // ── 3xx 弹操作 ───────────────────────────────────────────────────────
        SYS_SET_BULLET_SPEED => sys_set_bullet_speed(task, ctx),
        SYS_SET_BULLET_ANGLE => sys_set_bullet_angle(task, ctx),
        SYS_TURN_BULLET => sys_turn_bullet(task, ctx),
        SYS_SET_BULLET_VEL => sys_set_bullet_vel(task, ctx),
        SYS_SET_BULLET_ANG_VEL => sys_set_bullet_ang_vel(task, ctx),
        SYS_SET_BULLET_ACCEL => sys_set_bullet_accel(task, ctx),
        SYS_SET_BULLET_GRAVITY => sys_set_bullet_gravity(task, ctx),
        SYS_STOP_BULLET_FX => sys_stop_bullet_fx(task, ctx),
        SYS_AIM_BULLET_AT_PLAYER => sys_aim_bullet_at_player(task, ctx),

        // ── 4xx 敌运动 ───────────────────────────────────────────────────────
        SYS_MOVE_ENEMY_TO => sys_move_enemy_to(task, ctx),
        // ── 敌人运动动词族 410-421（T4）────────────────────────────────────────
        SYS_MOVE_VEL => sys_move_vel(task, ctx),
        SYS_MOVE_VEL_XY => sys_move_vel_xy(task, ctx),
        SYS_MOVE_ANGLE => sys_move_angle(task, ctx),
        SYS_MOVE_SPEED => sys_move_speed(task, ctx),
        SYS_SET_ANM_STATE => sys_set_anm_state(task, ctx),
        // ── 敌判定族 440-443（boss 换段刀）──────────────────────────────────────
        SYS_SET_INVULN => sys_set_invuln(task, ctx),
        SYS_SET_HITBOX => sys_set_hitbox(task, ctx),
        SYS_SET_HURTBOX => sys_set_hurtbox(task, ctx),
        SYS_SET_ENEMY_FLAG => sys_set_enemy_flag(task, ctx),
        SYS_SET_SELF_INT => sys_set_self_int(task, ctx),
        SYS_SET_SELF_FX => sys_set_self_fx(task, ctx),

        // ── 5xx 局面·记账·道具 ──────────────────────────────────────────────
        SYS_ADD_SCORE => sys_add_score(task, ctx),
        // B20 三件套：`d` 是脚本 push 上来的**任意 i32**，故一律先 `saturating_add`
        // 再 `clamp` —— 裸 `+`（如 `255i32 + i32::MAX`）在 debug 下溢出 panic，而
        // "调用方给坏参数"是 P4-b（确定性安全结果），不是 P4-c（引擎自身 bug 就地炸）。
        SYS_ADD_LIVES => sys_add_lives(task, ctx),
        SYS_ADD_BOMBS => sys_add_bombs(task, ctx),
        SYS_ADD_POWER => sys_add_power(task, ctx),
        // 敌人死亡效果四件（520-530）：一律经 `WorldBody` 的 handle 写 API（P1：调用方
        // 永不直接摸池内存），self owner 必须是敌（misuse → Fault，同 `move_enemy_to`）。
        SYS_DROP_CLEAR => sys_drop_clear(task, ctx),
        SYS_DROP_ADD => sys_drop_add(task, ctx),
        SYS_DROP_ITEMS => sys_drop_items(task, ctx),
        SYS_KILL_ALL_ENEMIES => sys_kill_all_enemies(task, ctx),
        SYS_DIE => sys_die(task, ctx),
        SYS_CLEAR_BULLETS_AT => sys_clear_bullets_at(task, ctx),
        SYS_CLEAR_BULLETS => sys_clear_bullets(task, ctx),
        SYS_BGM => sys_anchor_u16(task, ctx, AnchorKind::Bgm),
        SYS_BG => sys_anchor_u16(task, ctx, AnchorKind::Bg),
        SYS_BG_PHASE => sys_anchor_u16(task, ctx, AnchorKind::BgPhase),
        SYS_TIME_STOP_PLAYER => sys_time_stop_player(task, ctx),
        SYS_SET_PLAYER_HITBOX => sys_set_player_hitbox(task, ctx),

        // ── 6xx shooter ──────────────────────────────────────────────────────
        // Shooter 配置面 600-652（参数**逆序弹出**，照 `sys_move_enemy_to`；`id` 是首参、
        // 故最后弹）。每条都是"弹完全部参数 → 取槽（越界即 no-op）→ 写字段"三段式。
        SYS_SH_RESET => sys_sh_reset(task, ctx),
        SYS_SH_SPRITE => sys_sh_sprite(task, ctx),
        SYS_SH_OFFSET => sys_sh_offset(task, ctx),
        SYS_SH_OFFSET_ABS => sys_sh_offset_abs(task, ctx),
        SYS_SH_OFFSET_RAD => sys_sh_offset_rad(task, ctx),
        SYS_SH_DIST => sys_sh_dist(task, ctx),
        SYS_SH_ANGLE => sys_sh_angle(task, ctx),
        SYS_SH_SPEED => sys_sh_speed(task, ctx),
        SYS_SH_COUNT => sys_sh_count(task, ctx),
        SYS_SH_AIM => sys_sh_aim(task, ctx),
        SYS_SH_RING => sys_sh_ring(task, ctx),
        SYS_SH_XFORM => sys_sh_xform(task, ctx),
        SYS_SH_TASK => sys_sh_task(task, ctx),
        SYS_SH_REQ => sys_sh_req(task, ctx),
        SYS_SH_FIRE => sys_sh_fire(task, ctx),

        // ── 7xx 控制·事件·符卡 ─────────────────────────────────────────────
        SYS_ACCUM_START | SYS_ACCEL_START | SYS_HERMITE_START | SYS_ACCUM_VALUE
        | SYS_ACCEL_VALUE | SYS_HERMITE_VALUE => sys_interp(task, ctx, no),
        SYS_GET_VAR => sys_get_var(task, ctx),
        SYS_SET_VAR => sys_set_var(task, ctx),
        SYS_PULSE_SIGNAL => sys_pulse_signal(task, ctx),
        SYS_EMIT_REQ => sys_emit_req(task, ctx),
        SYS_FX_AT => sys_fx_at(task, ctx),
        SYS_FX_ON => sys_fx_on(task, ctx),
        SYS_STAGE_CLEAR => sys_stage_clear(task, ctx),
        SYS_BOSS_SET => sys_boss_set(task, ctx),
        SYS_SPELL_BEGIN => sys_spell_begin(task, ctx),
        SYS_SPELL_END => sys_spell_end(task, ctx),

        // ── 8xx 激光（Task 4）───────────────────────────────────────────────
        SYS_LASER_CREATE => sys_laser_create(task, ctx),
        SYS_LASER_SPEED => sys_laser_speed(task, ctx),
        SYS_LASER_START => sys_laser_start(task, ctx),
        SYS_LASER_OMEGA => sys_laser_omega(task, ctx),
        SYS_LASER_ROTATE => sys_laser_rotate(task, ctx),
        SYS_LASER_AIM => sys_laser_aim(task, ctx),
        SYS_LASER_ANCHOR => sys_laser_anchor(task, ctx),
        SYS_LASER_ORIGIN => sys_laser_origin(task, ctx),
        SYS_LASER_CANCEL => sys_laser_cancel(task, ctx),
        SYS_LASER_ALIVE => sys_laser_alive(task, ctx),
        SYS_LASER_X | SYS_LASER_Y | SYS_LASER_ANGLE | SYS_LASER_NEAR | SYS_LASER_FAR => {
            sys_laser_read(task, ctx, no)
        }

        _ => Err(FAULT_BAD_OP),
    }
}

/// 栈顶 i32 的低 16 位 → BAM（同 `OP_SINB`/`OP_COSB` 的取角惯例）。
#[inline]
fn bam(raw: i32) -> Angle {
    Angle((raw as u32 & 0xFFFF) as u16)
}

#[cfg(test)]
mod tests;
