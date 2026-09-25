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
// （非 ENEMY）→ Fault，悬垂句柄/`easing>=8` 的 P4-b 校验全在世界层做过，本层不重复计数。

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
/// **存在的理由是跨 crate**（`dispatch` 是 `pub(crate)`、102 条结构测试的表是 `cfg(test)`，
/// 编译器 crate 两个都够不着）：`stg-ecl-compiler` 的 `builtins::BUILTINS` 要能断言
/// "`is_op == false` 的条目，其 `syscall` 字段装的确实是个会被派发的号"。见
/// `builtins.rs::builtin_dispatch_kind_matches_what_the_field_holds`。
///
/// **与 `dispatch` 的同步靠测试押运，不靠自律**：`syscall_whitelist_matches_the_frozen_table`
/// 断言"全 `u16` 域里为真的号恰好是那 102 条"，漏一条/多一条即红。
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

/// `SYS_SPELL_TIMER`（130）：owner 非 ENEMY → 直接押 -1（同 `SYS_SELF_HP` 误用降级口径，
/// 不 Fault——读族误用是"确定性安全结果"而非脚本作者违约）；敌 → 押
/// `spell_frames_left_of`（世界侧核已处理"无绑定 → -1"）。
fn sys_spell_timer(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    if task.owner_kind != OWNER_ENEMY {
        return push(task, -1);
    }
    let h = EnemyHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    };
    let frames_left = ctx.body.spell_frames_left_of(h);
    push(task, frames_left)
}

// ── 敌号编解码（敌句柄打包刀 2026-07-31；六处产/消口共用，别各写各的）──────────
//
// 打包编码 [`crate::enemy::pack_handle`] 2026-09-15 起公开于 `enemy` 模块（stg-rl
// 观测编码复用同一约定），本文件只保留解码侧 `resolve_enemy_handle`。

/// 打包敌号 → 活槽索引；`None` = 无效（负值哨兵 / 越界 / 死槽 / **generation 不符**）。
///
/// 判据 = `packed >= 0 && idx < CAP && is_alive(idx) && (generation[idx] & 0x7FFF) == g`。
/// 前三条与打包前逐字相同（含 dying 语义：`is_alive` 是占用位，`ENEMY_DYING` 只是 flag，
/// 槽活到相位 9 才回收），第四条是本刀新增的那条——槽回收再复用之后旧句柄不再指向占了
/// 这个槽的另一只敌。
///
/// ⚠️ **两边都要 `& 0x7FFF`**：池的 `generation` 是完整 `u16` 而句柄只带低 15 位。
/// 拿 `generation[idx] == g` 直接比，gen 一旦越过 `0x7FFF` 就永远比不中——那会变成
/// "敌活着但所有读口都说它没了"，且要跑 32768 次同槽复用才撞得到（钉在
/// `packed_handle_stays_non_negative_and_resolves_with_a_high_generation`）。
fn resolve_enemy_handle(packed: i32, ctx: &VmCtx) -> Option<usize> {
    if packed < 0 {
        return None;
    }
    let idx = (packed & 0xFFFF) as usize;
    let g = ((packed >> 16) & 0x7FFF) as u16;
    if idx < crate::enemy::EnemyPool::CAP
        && ctx.body.enemies.is_alive(idx)
        && (ctx.body.enemies.generation[idx] & 0x7FFF) == g
    {
        Some(idx)
    } else {
        None
    }
}

/// 打包激光句柄 → 栈上 i32（编码同敌号：`((gen & 0x7FFF) << 16) | index`，NULL → -1）。
fn pack_laser_handle(h: LaserHandle) -> i32 {
    if h == LaserHandle::NULL {
        -1
    } else {
        (((h.generation & 0x7FFF) as i32) << 16) | h.index as i32
    }
}

/// 打包激光号 → 活槽句柄；`None` = 无效（负哨兵 / 越界 / 死槽 / **generation 不符**）。
/// 判据与 [`resolve_enemy_handle`] 逐条同款（两边都 `& 0x7FFF`）；重建时取池里的**完整
/// u16 代际**——相位 5 的挂靠比较用的是完整代际，低 15 位复刻会让高代际静默脱钩（I-3）。
fn resolve_laser_handle(packed: i32, ctx: &VmCtx) -> Option<LaserHandle> {
    if packed < 0 {
        return None;
    }
    let idx = (packed & 0xFFFF) as usize;
    let g = ((packed >> 16) & 0x7FFF) as u16;
    if idx < LaserPool::CAP
        && ctx.body.lasers.is_alive(idx)
        && (ctx.body.lasers.generation[idx] & 0x7FFF) == g
    {
        Some(LaserHandle {
            index: idx as u16,
            generation: ctx.body.lasers.generation[idx],
        })
    } else {
        None
    }
}

/// `SYS_ENEMY_HP`（100；A5 补遗）：1 参 `handle`（**打包敌号**，含 generation——槽复用后
/// 旧句柄可辨，见 [`resolve_enemy_handle`]；无效句柄仍是读族误用降级口径，不 Fault）。
/// 活敌返当前 hp；越界/死槽/gen 不符/负值一律 -1——stage 编排"等 boss 死"的轮询原语。
fn sys_enemy_hp(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let handle = pop(task)?;
    let hp = match resolve_enemy_handle(handle, ctx) {
        Some(idx) => ctx.body.enemies.hp[idx],
        None => -1,
    };
    push(task, hp)
}

/// `SYS_ENEMY_X`(101)/`SYS_ENEMY_Y`(102) 共享的读法——存活判据逐字同 `sys_enemy_hp`
/// （同一个 [`resolve_enemy_handle`]；dying 仍算活），只有降级值不同（坐标无哨兵位可用
/// → `Fx::ZERO`，理由见 [`SYS_ENEMY_X`] 号表注释）。`want_y` 选轴：`false`=x，`true`=y。
fn sys_enemy_pos(task: &mut Task, ctx: &mut VmCtx, want_y: bool) -> Result<(), u8> {
    let handle = pop(task)?;
    let v = match resolve_enemy_handle(handle, ctx) {
        Some(idx) => {
            if want_y {
                ctx.body.enemies.y[idx]
            } else {
                ctx.body.enemies.x[idx]
            }
        }
        None => Fx::ZERO,
    };
    push(task, v.raw())
}

/// `SYS_ENEMY_ALIVE`(103)：存活判据**逐字同** `sys_enemy_hp`/`sys_enemy_pos`（同一个
/// [`resolve_enemy_handle`]：负句柄/越界/死槽/gen 不符 → 0；`ENEMY_DYING` 仍算活——
/// `is_alive` 是存活位，dying 只是 flag，槽活到相位 9 才回收），只是把那个判据**本身**
/// 押出去而不是拿它选一个值。
///
/// 存在的理由见 [`SYS_ENEMY_ALIVE`] 号表注释：`enemy_hp(e) != -1` 这个旧探针在"活敌血量
/// 恰为 −1"那一格会误判，专用口没有这条缝。
fn sys_enemy_alive(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let handle = pop(task)?;
    let alive = resolve_enemy_handle(handle, ctx).is_some();
    push(task, if alive { 1 } else { 0 })
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

/// 取本任务的第 `id` 个 shooter（可变）。`id` 越界 → `None` + `contract_viol` + `BAD_ARGS`
/// （P4-b：no-op，不 Fault——见 600-660 号表注释）。`id` 是脚本给的任意 `i32`，负数与超界同处置。
///
/// **必须在参数全部 `pop` 完之后再调**：栈效应与成功路径一致（同 `sys_emit_req` 的"先弹后验"
/// 口径），否则越界腿会给下一条指令留下垃圾栈。
fn shooter_mut<'a>(
    id: i32,
    task_index: u16,
    ctx: &'a mut VmCtx<'_>,
) -> Option<&'a mut ShooterSlot> {
    if id < 0 || id as usize >= SHOOTERS_PER_TASK {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return None;
    }
    Some(&mut ctx.tasks.shooters[task_index as usize][id as usize])
}

/// 号表派发。坏号/参数不足 → `Err(fault_code)`（由 `vm::exec` 转 `Exec::Fault`）。
pub(crate) fn dispatch(no: u16, task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    match no {
        // ── 0xx `$` 引擎变量 ──────────────────────────────────────────────────
        SYS_FRAME => push(task, ctx.frame as i32),
        SYS_PLAYER_X => push(task, ctx.body.players[0].x.raw()),
        SYS_PLAYER_Y => push(task, ctx.body.players[0].y.raw()),
        SYS_SELF_ENEMY => {
            // 非敌押 -1 而非 0：打包敌号 0 合法（spec §5.2），与族内其它变量「非敌读 0」有意不同。
            let v = if task.owner_kind == OWNER_ENEMY {
                crate::enemy::pack_handle(EnemyHandle {
                    index: task.owner_index,
                    generation: task.owner_gen,
                })
            } else {
                -1
            };
            push(task, v)
        }
        SYS_SELF_X => {
            let (x, _) = self_pos(task, ctx);
            push(task, x.raw())
        }
        SYS_SELF_Y => {
            let (_, y) = self_pos(task, ctx);
            push(task, y.raw())
        }
        // ── $self_* 速度引擎变量 022-025（T5）──────────────────────────────────
        SYS_SELF_VX => {
            let (vx, _, _, _) = self_vel(task, ctx);
            push(task, vx)
        }
        SYS_SELF_VY => {
            let (_, vy, _, _) = self_vel(task, ctx);
            push(task, vy)
        }
        SYS_SELF_SPEED => {
            let (_, _, sp, _) = self_vel(task, ctx);
            push(task, sp)
        }
        SYS_SELF_ANGLE => {
            let (_, _, _, a) = self_vel(task, ctx);
            push(task, a)
        }
        SYS_SELF_HP => {
            let hp = self_hp(task, ctx);
            push(task, hp)
        }
        SYS_SELF_HP_MAX => {
            let hp_max = self_hp_max(task, ctx);
            push(task, hp_max)
        }
        SYS_SELF_AGE => {
            let age = ctx.frame.wrapping_sub(task.born_frame) as i32;
            push(task, age)
        }

        // ── 1xx 查询 ─────────────────────────────────────────────────────────
        SYS_ENEMY_HP => sys_enemy_hp(task, ctx),
        // ── 敌坐标读口 101/102（敌坐标读口刀）──────────────────────────────────
        SYS_ENEMY_X => sys_enemy_pos(task, ctx, false),
        SYS_ENEMY_Y => sys_enemy_pos(task, ctx, true),
        // ── 探活读口 103（探活读口刀）────────────────────────────────────────
        SYS_ENEMY_ALIVE => sys_enemy_alive(task, ctx),
        SYS_NEAREST_ENEMY => {
            let y = pop(task)?;
            let x = pop(task)?;
            // 世界侧返的本来就是**完整句柄**——打包刀之前这里只押 `h.index`、把 gen 丢了。
            let handle = match ctx.body.nearest_enemy(Fx::from_raw(x), Fx::from_raw(y)) {
                Some(h) => crate::enemy::pack_handle(h),
                None => -1,
            };
            push(task, handle)
        }
        SYS_AIM_PLAYER_ANGLE => sys_aim_player_angle(task, ctx),
        SYS_SPELL_TIMER => sys_spell_timer(task, ctx),
        SYS_SPELL_RESULT => {
            let slot = pop(task)?;
            let v = match usize::try_from(slot) {
                Ok(s) if s < crate::boss::MAX_BOSSES => ctx.body.spell_last_result[s] as i32,
                _ => {
                    ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                    ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                    0
                }
            };
            push(task, v)
        }
        // ── 数学/查询面 110/140/141（参数**逆序弹出**，照 `sys_move_enemy_to`）────────────
        SYS_ATAN2 => {
            let x = pop(task)?;
            let y = pop(task)?;
            let a = crate::math::cordic::atan2(Fx::from_raw(y), Fx::from_raw(x));
            push(task, a.raw() as i32)
        }
        SYS_DIST => {
            let dy = pop(task)?;
            let dx = pop(task)?;
            // Q32.32 → 开根 → Q16.16（见 SYS_DIST 号表注释的值域论证）。`.min()` 不是
            // 冗余：极端字面量（`dx = dy = i32::MAX`）能让开根结果超 i32 上限，裸 `as i32`
            // 会回绕成负距离——饱和降级换掉那个静默错值。
            let d2 = crate::math::geom::len_sq(Fx::from_raw(dx), Fx::from_raw(dy));
            let root = crate::math::isqrt::isqrt(d2 as u64).min(i32::MAX as u32);
            let d = Fx::from_raw(root as i32);
            push(task, d.raw())
        }
        SYS_RAND_RANGE => sys_rand_range(task, ctx),

        // ── 2xx 造物 ─────────────────────────────────────────────────────────
        SYS_CREATE_BULLET => sys_create_bullet(task, ctx),
        SYS_CREATE_BULLETS_BATCH => sys_create_bullets_batch(task, ctx),
        SYS_SPAWN_ENEMY => sys_spawn_enemy(task, ctx),
        SYS_DROP_ITEM => sys_drop_item(task, ctx),

        // ── 3xx 弹操作 ───────────────────────────────────────────────────────
        SYS_SET_BULLET_SPEED => {
            let h = self_bullet_handle(task)?;
            let speed = pop(task)?;
            ctx.body.set_bullet_speed(h, Fx::from_raw(speed));
            Ok(())
        }
        SYS_SET_BULLET_ANGLE => {
            let h = self_bullet_handle(task)?;
            let a = pop(task)?;
            ctx.body.set_bullet_angle(h, bam(a));
            Ok(())
        }
        SYS_TURN_BULLET => {
            let h = self_bullet_handle(task)?;
            let d = pop(task)?;
            ctx.body.turn_bullet(h, bam(d));
            Ok(())
        }
        SYS_SET_BULLET_VEL => {
            let h = self_bullet_handle(task)?;
            let vy = pop(task)?;
            let vx = pop(task)?;
            ctx.body
                .set_bullet_vel(h, Fx::from_raw(vx), Fx::from_raw(vy));
            Ok(())
        }
        SYS_SET_BULLET_ANG_VEL => {
            let h = self_bullet_handle(task)?;
            let w = pop(task)?;
            ctx.body.set_bullet_ang_vel(h, w as i16);
            Ok(())
        }
        SYS_SET_BULLET_ACCEL => {
            let h = self_bullet_handle(task)?;
            let a = pop(task)?;
            ctx.body.set_bullet_accel(h, Fx::from_raw(a));
            Ok(())
        }
        SYS_SET_BULLET_GRAVITY => {
            let h = self_bullet_handle(task)?;
            let ay = pop(task)?;
            let ax = pop(task)?;
            ctx.body
                .set_bullet_gravity(h, Fx::from_raw(ax), Fx::from_raw(ay));
            Ok(())
        }
        SYS_STOP_BULLET_FX => {
            let h = self_bullet_handle(task)?;
            ctx.body.stop_bullet_fx(h);
            Ok(())
        }
        SYS_AIM_BULLET_AT_PLAYER => {
            let h = self_bullet_handle(task)?;
            let d = pop(task)?;
            ctx.body.aim_bullet_at_player(h, bam(d));
            Ok(())
        }

        // ── 4xx 敌运动 ───────────────────────────────────────────────────────
        SYS_MOVE_ENEMY_TO => sys_move_enemy_to(task, ctx),
        // ── 敌人运动动词族 410-421（T4）────────────────────────────────────────
        SYS_MOVE_VEL => sys_move_vel(task, ctx),
        SYS_MOVE_VEL_XY => sys_move_vel_xy(task, ctx),
        SYS_MOVE_ANGLE => sys_move_angle(task, ctx),
        SYS_MOVE_SPEED => sys_move_speed(task, ctx),
        SYS_SET_ANM_STATE => sys_set_anm_state(task, ctx),
        // ── 敌判定族 440-443（boss 换段刀）──────────────────────────────────────
        SYS_SET_INVULN => {
            let h = self_enemy_handle(task)?;
            let n = pop(task)?;
            let Ok(frames) = u16::try_from(n) else {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                return Ok(());
            };
            ctx.body.set_enemy_invuln(h, frames);
            Ok(())
        }
        SYS_SET_HITBOX => {
            let h = self_enemy_handle(task)?;
            let r = pop(task)?;
            ctx.body.set_enemy_hitbox(h, Fx::from_raw(r));
            Ok(())
        }
        SYS_SET_HURTBOX => {
            let h = self_enemy_handle(task)?;
            let r = pop(task)?;
            ctx.body.set_enemy_hurtbox(h, Fx::from_raw(r));
            Ok(())
        }
        SYS_SET_ENEMY_FLAG => {
            let h = self_enemy_handle(task)?;
            let on = pop(task)?;
            let mask = pop(task)?;
            const SETTABLE: i32 =
                (crate::enemy::ENEMY_NO_BODY | crate::enemy::ENEMY_KILLALL_EXEMPT) as i32;
            if mask == 0 || mask & !SETTABLE != 0 {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                return Ok(());
            }
            ctx.body.set_enemy_flags(h, mask as u8, on != 0);
            Ok(())
        }

        // ── 5xx 局面·记账·道具 ──────────────────────────────────────────────
        SYS_ADD_SCORE => {
            let d = pop(task)?;
            let s = &mut ctx.body.players[0].score;
            *s = if d >= 0 {
                s.saturating_add(d as u64)
            } else {
                s.saturating_sub(d.unsigned_abs() as u64)
            };
            Ok(())
        }
        // B20 三件套：`d` 是脚本 push 上来的**任意 i32**，故一律先 `saturating_add`
        // 再 `clamp` —— 裸 `+`（如 `255i32 + i32::MAX`）在 debug 下溢出 panic，而
        // "调用方给坏参数"是 P4-b（确定性安全结果），不是 P4-c（引擎自身 bug 就地炸）。
        SYS_ADD_LIVES => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            p.lives = (p.lives as i32).saturating_add(d).clamp(0, u8::MAX as i32) as u8;
            Ok(())
        }
        SYS_ADD_BOMBS => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            p.bombs = (p.bombs as i32)
                .saturating_add(d)
                .clamp(0, crate::player::STOP_STOCK_MAX as i32) as u8;
            Ok(())
        }
        SYS_ADD_POWER => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            // 上钳 POWER_MAX(400) 而非 u16::MAX —— 见 SYS_ADD_POWER 号表注释。
            p.power = (p.power as i32)
                .saturating_add(d)
                .clamp(0, crate::items::POWER_MAX as i32) as u16;
            Ok(())
        }
        // 敌人死亡效果四件（520-530）：一律经 `WorldBody` 的 handle 写 API（P1：调用方
        // 永不直接摸池内存），self owner 必须是敌（misuse → Fault，同 `move_enemy_to`）。
        SYS_DROP_CLEAR => {
            let h = self_enemy_handle(task)?;
            ctx.body.clear_enemy_drops(h);
            Ok(())
        }
        SYS_DROP_ADD => {
            let h = self_enemy_handle(task)?;
            // 逆序弹出（模块文档"参数传递约定"）：`drop_add(type, n)` 故先 `n` 后 `type`。
            let n = pop(task)?;
            let item_type = pop(task)?;
            ctx.body.add_enemy_drop(h, item_type, n);
            Ok(())
        }
        SYS_DROP_ITEMS => {
            let h = self_enemy_handle(task)?;
            ctx.body.spill_enemy_drops(h, ctx.tables);
            Ok(())
        }
        SYS_KILL_ALL_ENEMIES => {
            let mode = pop(task)?;
            let die = match mode {
                m if m == crate::enemy::KILL_SILENT as i32 => false,
                m if m == crate::enemy::KILL_DIE as i32 => true,
                _ => {
                    ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                    ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                    return Ok(());
                }
            };
            let except = (task.owner_kind == OWNER_ENEMY).then_some(EnemyHandle {
                index: task.owner_index,
                generation: task.owner_gen,
            });
            ctx.body.kill_all_enemies(except, die, ctx.tables);
            Ok(())
        }
        SYS_DIE => {
            let h = self_enemy_handle(task)?;
            ctx.body.kill_enemy_by_handle(h, ctx.tables);
            Ok(())
        }
        SYS_CLEAR_BULLETS_AT => {
            let stars = pop(task)?;
            let r = pop(task)?;
            let y = pop(task)?;
            let x = pop(task)?;
            ctx.body.create_field(crate::field::clear_field_at(
                Fx::from_raw(x),
                Fx::from_raw(y),
                Fx::from_raw(r),
                stars != 0,
            ));
            Ok(())
        }
        SYS_CLEAR_BULLETS => {
            ctx.body
                .create_field(crate::field::fullscreen_clear_field());
            Ok(())
        }
        SYS_BGM => sys_anchor_u16(task, ctx, AnchorKind::Bgm),
        SYS_BG => sys_anchor_u16(task, ctx, AnchorKind::Bg),
        SYS_BG_PHASE => sys_anchor_u16(task, ctx, AnchorKind::BgPhase),
        SYS_TIME_STOP_PLAYER => {
            let n = pop(task)?;
            // D19 判例：越界 → P4-b 整条 no-op + 计数 + BAD_ARGS，不钳位、不 Fault。
            let Ok(frames) = u16::try_from(n) else {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                return Ok(());
            };
            ctx.body.freeze_left[1] = frames;
            Ok(())
        }

        // ── 6xx shooter ──────────────────────────────────────────────────────
        // Shooter 配置面 600-652（参数**逆序弹出**，照 `sys_move_enemy_to`；`id` 是首参、
        // 故最后弹）。每条都是"弹完全部参数 → 取槽（越界即 no-op）→ 写字段"三段式。
        SYS_SH_RESET => {
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                *s = ShooterSlot::default();
            }
            Ok(())
        }
        SYS_SH_SPRITE => {
            // 表层是 (id, shape, color) 三参，但 shape/color 在**编译期**已折叠成单个
            // appearance 值（`builtins::fold_start("sh_sprite") == 1`），故这里只弹两个。
            // appearance 在册与否留到开火时查（同 `fire` 的先验后建）。
            //
            // **收窄必须保号越界性**：`try_from` 失败（负值 / 超 u16）一律落到 `u16::MAX`
            // ——它远超表长，T3 开火时照样查不到 ⇒ 越界值进来、越界值出去。
            // 曾经写成 `clamp(0, u16::MAX)`，把负值钳成 **0**，而 `appearances[0]` 是在册
            // 且 valid 的格子 ⇒ 越界值被偷偷洗成合法值，开火时再也拒不掉，和上面那句
            // "留到开火时查"的承诺直接矛盾（复审 ①）。`fire` 侧同样的脚本值是 Fault
            // （`appearances.get(负 as usize)` → `None`），两条路不该有这种差别。
            // 可达性是真的：typeck 的形色判据只在两参**都是编译期常量**时施加，
            // `sh_sprite(0, 16, c)` 里 `c` 是变量时判据跳过，`c = -20` 就折叠成 -4。
            let appearance = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.appearance = u16::try_from(appearance).unwrap_or(u16::MAX);
            }
            Ok(())
        }
        SYS_SH_OFFSET => {
            let y = pop(task)?;
            let x = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.off_x = Fx::from_raw(x);
                s.off_y = Fx::from_raw(y);
                s.flags &= !SH_ABS_OFFSET; // 相对模式：清标志（与 sh_offset_abs 互为反向）
            }
            Ok(())
        }
        SYS_SH_OFFSET_ABS => {
            let y = pop(task)?;
            let x = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.off_x = Fx::from_raw(x);
                s.off_y = Fx::from_raw(y);
                s.flags |= SH_ABS_OFFSET;
            }
            Ok(())
        }
        SYS_SH_OFFSET_RAD => {
            let r = pop(task)?;
            let ang = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.polar_ang = bam(ang);
                s.polar_r = Fx::from_raw(r);
            }
            Ok(())
        }
        SYS_SH_DIST => {
            let d = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.dist = Fx::from_raw(d);
            }
            Ok(())
        }
        SYS_SH_ANGLE => {
            let step = pop(task)?;
            let angle0 = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.angle0 = bam(angle0);
                s.angle_step = bam(step);
            }
            Ok(())
        }
        SYS_SH_SPEED => {
            let step = pop(task)?;
            let speed0 = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.speed0 = Fx::from_raw(speed0);
                s.speed_step = Fx::from_raw(step);
            }
            Ok(())
        }
        SYS_SH_COUNT => {
            let n_speed = pop(task)?;
            let n_angle = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                // P4-b：脚本给的任意 i32，先钳进 [0,255] 再存（裸 `as u8` 会让负数回绕）。
                s.n_angle = n_angle.clamp(0, u8::MAX as i32) as u8;
                s.n_speed = n_speed.clamp(0, u8::MAX as i32) as u8;
            }
            Ok(())
        }
        SYS_SH_AIM => {
            let on = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                if on != 0 {
                    s.flags |= SH_AIMED;
                } else {
                    s.flags &= !SH_AIMED;
                }
            }
            Ok(())
        }
        SYS_SH_RING => {
            let on = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                if on != 0 {
                    s.flags |= SH_RING;
                } else {
                    s.flags &= !SH_RING;
                }
            }
            Ok(())
        }
        SYS_SH_XFORM => {
            // 表层 `XformRef` 在 codegen 里降低成 `(off, cnt)` **两个**栈值。`cnt == 0` 即无
            // xform。**此处不校验区间**——`off + cnt*3 ≤ LOCALS` 在开火时查（与 `fire` 同
            // 口径：`fire` 也是在建弹那一刻才查 `LOCALS` 边界）。这里只做**存储收窄**的
            // 饱和钳，防负值/超宽裸 `as` 回绕成一个看似合法的小区间。
            let cnt = pop(task)?;
            let off = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.xform_off = off.clamp(0, u16::MAX as i32) as u16;
                s.xform_cnt = cnt.clamp(0, u8::MAX as i32) as u8;
            }
            Ok(())
        }
        SYS_SH_TASK => {
            // 表层 `SubRef` 降低成一个栈值（sub 号或 -1）。**此处不校验号是否在册**——与
            // `fire` 的"先验后建"不同，因为设的时候还没建弹；校验在开火时做。
            // 超 `u16` 的号同样降级成"不挂"（P4-b，确定性安全结果）。
            let sub = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.task_script = if sub < 0 {
                    SH_NO_TASK
                } else {
                    u16::try_from(sub).unwrap_or(SH_NO_TASK)
                };
            }
            Ok(())
        }
        SYS_SH_REQ => {
            let req_id = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.on_fire_req = req_id.clamp(0, u16::MAX as i32) as u16;
            }
            Ok(())
        }
        SYS_SH_FIRE => sys_sh_fire(task, ctx),

        // ── 7xx 控制·事件·符卡 ─────────────────────────────────────────────
        SYS_GET_VAR => {
            let slot = pop(task)? as u16;
            let v = ctx.body.get_var(slot);
            push(task, v)
        }
        SYS_SET_VAR => {
            let val = pop(task)?;
            let slot = pop(task)? as u16;
            // globals 段纪律（甲案，M1.5）：脚本写系统段（slot < GLOBALS_SYS_SEGMENT）→
            // no-op + contract_viol 计数（P4-b 确定性安全结果，不 Fault——见
            // `world::GLOBALS_SYS_SEGMENT` 文档）。世界 API `set_var` 不经此门，见调用方。
            if slot < crate::world::GLOBALS_SYS_SEGMENT {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
            } else {
                ctx.body.set_var(slot, val);
            }
            Ok(())
        }
        SYS_PULSE_SIGNAL => {
            let ch = pop(task)?;
            ctx.body.pulse_signal(ch as usize);
            Ok(())
        }
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

/// 开火（SYS 660）：按 spec §10 的七步用发射器槽 `id` 的参数造弹。1 参、**无返回值**。
///
/// **为什么开火循环住这儿而不是扩 `create_bullets_batch`**：P1——world 不知道"任务"存在，
/// 没法逐颗挂 `task_script`（`fire` 的挂任务也因此发生在 ECL 层）。代价是网格循环有了
/// 第二份实现，故 `shooter_fan_matches_batch_with_centering_compensation` 那条等价测试
/// 是**必需**而非顺带的。
///
/// 校验序照抄 `sys_create_bullet` 的"先验后建"：退化网格（P4-b no-op）→ appearance
/// （Fault）→ xform 区间（Fault）→ 挂弹任务号（Fault）——一切拒绝都发生在任何世界写之前。
fn sys_sh_fire(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let id = pop(task)?;
    // 先取一份**拷贝**：后面要可变借用 `ctx.body`/`ctx.tasks` 建弹派任务，不能持着
    // `shooters` 的引用。顺带一层安全网——`tasks.spawn` 的"复用槽写满"会把新槽的
    // shooter 抹成默认，拿引用读到一半被抹是个隐蔽的自伤。
    let Some(sh) = shooter_mut(id, ctx.self_index, ctx).map(|s| *s) else {
        return Ok(()); // 越界 id：`shooter_mut` 已记 contract_viol + BAD_ARGS（P4-b）
    };

    // ① 退化网格（对齐 `create_bullets_batch` 的既有口径：不发 + contract_viol，不 Fault）。
    let total = sh.n_angle as u32 * sh.n_speed as u32;
    if sh.n_angle == 0 || sh.n_speed == 0 || total > crate::bullets::BulletPool::CAP as u32 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // ② 外观：越界/空格一律拒（同 `fire`——放行会造出"有判定但画面上什么都没有"的隐形弹，
    //    而校验和不关心贴图内容，金向量/冒烟都抓不到它）。T2 的 `sh_sprite` 收窄之所以
    //    必须保号越界性，就是为了让这一步还拒得掉。
    let tables = ctx.tables;
    let Some(cfg) = tables.appearances.get(sh.appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }
    let (sprite, radius) = (cfg.sprite, cfg.radius);

    // ③ xform 解包（照抄 `sys_create_bullet` 的 LOCALS 边界校验与解包循环）。数据住
    //    **本任务的 locals**，由 codegen 在 sub 入口一次性 staging、`slots` 的调用图着色
    //    保证区间不被复用 ⇒ 存 `(off, cnt)`、开火时才读是安全的（跨帧亦然，有测试钉着）。
    //    存储侧已是 u8/u16，负值不可达；只剩长度与区间两条。
    let mut xform_buf = [XformSlot::default(); 16];
    let xform_len: usize = if sh.xform_cnt == 0 {
        0
    } else {
        let cnt = sh.xform_cnt as usize;
        if cnt > 16 {
            return Err(FAULT_BAD_OP);
        }
        let off = sh.xform_off as usize;
        let Some(end) = off.checked_add(cnt * 3) else {
            return Err(FAULT_BAD_OP);
        };
        if end > LOCALS {
            return Err(FAULT_BAD_OP);
        }
        for (k, slot) in xform_buf.iter_mut().take(cnt).enumerate() {
            let base = off + k * 3;
            let word0 = task.locals[base] as u32;
            *slot = XformSlot {
                wait: (word0 >> 16) as u16,
                op: ((word0 >> 8) & 0xFF) as u8,
                _pad: 0,
                args: [task.locals[base + 1], task.locals[base + 2]],
            };
        }
        cnt
    };

    // ④ 挂弹任务号先验（照抄 `sys_create_bullet`：坏号 → Fault，零副作用，弹未建）。
    let task_sub: Option<SubId> = if sh.task_script == SH_NO_TASK {
        None
    } else {
        let sub = ctx.ecl.sub_id(sh.task_script).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async
            || ctx
                .ecl
                .param_types(sub)
                .is_none_or(|params| !params.is_empty())
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    };

    // ⑤ 原点（spec §10 步 2）：直角偏移与极坐标偏移**相加**（ZUN 626 明写 stacks），
    //    不是覆盖——判别腿见 `rect_and_polar_offsets_stack_rather_than_override`。
    let (bx, by) = if sh.flags & SH_ABS_OFFSET != 0 {
        (Fx::ZERO, Fx::ZERO)
    } else {
        self_pos(task, ctx)
    };
    let (px, py) = polar_to_vec(sh.polar_r, sh.polar_ang);
    // ⚠️ **裸 `+` 的溢出归 P4-c 域**（引擎自身 bug 才该触发的那一档：debug 溢出 panic、
    // release 回绕），不是 P4-b 的"调用方违约要降级"。三个 `Fx` 相加没有钳制——
    // `sh_offset(0, 32767.0fx, 0fx)` 配一个 x 非零的 owner 就能在 debug 下 panic。
    // **这不是本刀新立的立场**：`world.rs::create_bullets_batch` 的文档已经声明过
    // "两轴累加器：角度 BAM 回绕、速度 Fx 裸加（溢出 P4-c 域）"，而 ⑦ 的 `cur_speed +
    // sh.speed_step` 继承的正是那个累加器。本处（偏移相加）只是那条声明**没覆盖到的
    // 同类一处**——本仓第一次把两个**大坐标**相加（`fire`/`batch` 是把 x/y 直通不加），
    // 故在此显式记一笔，免得后人以为漏了 P4-b 校验。
    // 要改立场（譬如改成 `saturating_add` 或收窄 `sh_offset` 的入参）请连同
    // `create_bullets_batch` 那条一起改——单改一处会让两个发射路径口径分叉，
    // 而 `shooter_fan_matches_batch_with_centering_compensation` 那条等价测试正押着它们。
    let origin_x = bx + sh.off_x + px;
    let origin_y = by + sh.off_y + py;

    // ⑥ 基准角（spec §10 步 3）——**在这一刻**解析 aim。"瞄谁"走引擎唯一口径
    //    `WorldBody::aim_target`（F8 统一，2026-09-03）：从**出弹口**看过去最近的可瞄
    //    自机，一个都没有则回退 `players[0]` 的最后坐标（发射这条路必须产出一个角度，
    //    没有"不瞄"这个选项——那是弹上 setter 那半边的处置）。基点是出弹口而非 owner
    //    位置，判别腿见 `aim_is_measured_from_the_fire_origin_not_from_the_owner`。
    //    角度算术一律在 `i32` 里做、最后才回绕成 `Angle`（`Angle` 底层 u16，直接加会在
    //    debug 触发 overflow-checks）。
    let base: i32 = if sh.flags & SH_AIMED != 0 {
        let p = ctx.body.aim_target(origin_x, origin_y);
        let a = crate::math::cordic::atan2(
            ctx.body.players[p].y - origin_y,
            ctx.body.players[p].x - origin_x,
        );
        a.raw() as i32 + sh.angle0.raw() as i32
    } else {
        sh.angle0.raw() as i32
    };

    // ⑦ 网格：角度外层、速度内层（照抄 `create_bullets_batch` 的序 = 池槽分配序，I4）。
    //    fan  以基准方向为**中心**对称展开（D-7）：angle_i = base + i·step − ((n−1)·step)/2
    //    ring 不居中、逐颗算 (i×65536)/n 把余数**均摊**（精确闭合；`i ≤ 254` 故 i32 不溢出）
    //         且 `angle_step` 转义成**逐层**偏移。
    let ring = sh.flags & SH_RING != 0;
    let n_angle = sh.n_angle as i32;
    // **符号扩展是必须的，不是风格问题**（复审 ①，Critical）：`angle_step` 存的是 `Angle`
    // （底层 u16），脚本写 `sh_angle(0, 0deg, -6deg)` 存进去的是 `65536 − 1092`。
    // `i·step`/`j·step` 在 mod 65536 下不受零扩展影响，**但下面的 `/2` 不与 mod 65536 交换**：
    //   `((n−1)(s + 65536))/2 = ((n−1)s)/2 + (n−1)·32768`
    // `n` 奇数 ⇒ 多出项是 65536 的整数倍、无害；**`n` 偶数 ⇒ 多出半圈**，整把扇形被搬到
    // `base` 的正对面（形状还对，故只看"相邻差 step"的测试看不见它）。
    // 读侧符号扩展也正是本仓家规：`create_bullets_batch` 的形参就是 `angle_step: i16`、
    // `Angle::add_delta` 也收 i16。钉死在 `fan_centering_handles_even_ways_with_negative_step`。
    let step = sh.angle_step.raw() as i16 as i32;
    // 奇数路时 (n−1) 为偶数、除 2 精确；偶数路截断半个 BAM 单位（1/65536 圈，确定且可忽略）。
    let fan_center = ((n_angle - 1) * step) / 2;
    let mut created: u32 = 0;
    'grid: for i in 0..n_angle {
        let mut cur_speed = sh.speed0;
        for j in 0..sh.n_speed as i32 {
            let raw = if ring {
                base + (i * 65536) / n_angle + j * step
            } else {
                base + i * step - fan_center
            };
            let angle = bam(raw);
            // `dist`：逐颗沿**各自**角度推出去，不是整环平移（判别腿
            // `dist_pushes_each_bullet_along_its_own_angle`）。dist=0 时 polar_to_vec
            // 恒返 (0,0)，故不必分支。
            let (dx, dy) = polar_to_vec(sh.dist, angle);
            let (vx, vy) = polar_to_vec(cur_speed, angle);
            let init = BulletInit {
                x: origin_x + dx,
                y: origin_y + dy,
                vx,
                vy,
                speed: cur_speed,
                angle,
                ang_vel: 0,
                accel: Fx::ZERO,
                ax: Fx::ZERO,
                ay: Fx::ZERO,
                sprite,
                radius,
                delay: 0,
                life: 0xFFFF,
                flags: 0,
                grazed_by: 0,
                transform_head: crate::xform::XFORM_NONE,
                xform_wait: 0,
                xform_next: 0,
                born_frame: ctx.body.frame,
            };
            let handle = if xform_len == 0 {
                ctx.body.create_bullet(init)
            } else {
                ctx.body
                    .create_bullet_with_xform(init, &xform_buf[..xform_len])
            };
            if handle == BulletHandle::NULL {
                // 短路（同 `batch` 的 `'grid`）：同相位无回收 ⇒ 后续必然同败。
                // **故意不做 `batch` 那样的"剩余批量补计"**——shooter 的失败因不止池满
                // 一种（坏 xform 内容 → `create_bullet_with_xform` 记 contract_viol +
                // BAD_ARGS），批量补 `pool_full[POOL_BULLET]` 会张冠李戴。本颗的计数已由
                // `create_bullet*` 自己按真实原因记过，就到此为止。
                break 'grid;
            }
            created += 1;

            if let Some(task_sub) = task_sub {
                // 号已在 ④ 校验过在册；池满 → 静默计数（P4-a），**弹保留**、不 Fault（同 `fire`）。
                let pc0 = ctx
                    .ecl
                    .sub_meta(task_sub)
                    .expect("已在 ④ 校验过")
                    .code_entry();
                let owner = (OWNER_BULLET, handle.index, handle.generation);
                let parent = ctx.self_index + 1;
                if ctx
                    .tasks
                    .spawn(task_sub, pc0, owner, parent, ctx.frame)
                    .is_none()
                {
                    ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                        ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
                }
            }
            cur_speed = cur_speed + sh.speed_step;
        }
    }

    // ⑧ 开火请求（spec §8）：`args[3]` 是**实际**创建数而非请求数（池满时要能区分）。
    if sh.on_fire_req != 0 {
        ctx.body.emit_req(
            sh.on_fire_req,
            [
                origin_x.raw(),
                origin_y.raw(),
                sh.appearance as i32,
                created as i32,
                0,
                0,
            ],
        );
    }
    Ok(())
}

/// 5xx 族锚点写口的三种目标字段（`sys_anchor_u16` 判据）。
enum AnchorKind {
    Bgm,
    Bg,
    BgPhase,
}

/// 三锚共用：弹 1 参收窄 u16（越界 = P4-b no-op+viol，同 `sys_emit_req` 口径），再走 world 写口。
fn sys_anchor_u16(task: &mut Task, ctx: &mut VmCtx, kind: AnchorKind) -> Result<(), u8> {
    let v = pop(task)?;
    if !(0..=u16::MAX as i32).contains(&v) {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }
    match kind {
        AnchorKind::Bgm => ctx.body.set_bgm(v as u16),
        AnchorKind::Bg => ctx.body.set_bg(v as u16),
        AnchorKind::BgPhase => ctx.body.set_bg_phase(v as u16),
    }
    Ok(())
}

/// 栈顶 i32 的低 16 位 → BAM（同 `OP_SINB`/`OP_COSB` 的取角惯例）。
#[inline]
fn bam(raw: i32) -> Angle {
    Angle((raw as u32 & 0xFFFF) as u16)
}

// ── 8xx 激光族 handler（Task 4）──────────────────────────────────────────────
//
// 统一口径：`resolve_laser_handle` 失败就传 `LaserHandle::NULL`，由世界侧写 API no-op +
// 计一次 `contract_viol`——计数只发生在一处（计划 Step 2）。参数一律逆序弹出。

/// 出 `[0, u16::MAX]` → 钳位并把 `bad` 置真（一次 `laser()` 多坏字段只计一次违约）。
fn clamp_u16_count(v: i32, bad: &mut bool) -> u16 {
    if (0..=u16::MAX as i32).contains(&v) {
        v as u16
    } else {
        *bad = true;
        v.clamp(0, u16::MAX as i32) as u16
    }
}

/// `laser()`（800）：9 参逆序弹。`color ∈ 0..=15` 否则 Fault（先验后建，同 `fire` 坏外观）；
/// `warn/active/fade` 出 `[0,65535]` 钳位并计一次违约；形态一字段 `start=0`、
/// `end=start_len=len`、`speed=omega=0`、`flags=0`，其余派生字段交给 `create_laser`。
fn sys_laser_create(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let fade = pop(task)?;
    let active = pop(task)?;
    let warn = pop(task)?;
    let width = pop(task)?;
    let len = pop(task)?;
    let angle = pop(task)?;
    let y = pop(task)?;
    let x = pop(task)?;
    let color = pop(task)?;
    if !(0..=15).contains(&color) {
        return Err(FAULT_BAD_OP);
    }
    let mut bad = false;
    let warn = clamp_u16_count(warn, &mut bad);
    let active = clamp_u16_count(active, &mut bad);
    let fade = clamp_u16_count(fade, &mut bad);
    if bad {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
    }
    let init = LaserInit {
        ox: Fx::from_raw(x),
        oy: Fx::from_raw(y),
        angle: bam(angle),
        omega: 0,
        start: Fx::ZERO,
        end: Fx::from_raw(len),
        start_len: Fx::from_raw(len),
        speed: Fx::ZERO,
        width: Fx::from_raw(width),
        sprite: color as u16,
        warn,
        active,
        fade,
        timer: 0,
        state: 0,
        anchor_idx: ANCHOR_NONE,
        anchor_gen: 0,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        dx: Fx::ZERO,
        dy: Fx::ZERO,
        dang: 0,
        px: Fx::ZERO,
        py: Fx::ZERO,
        pang: Angle::ZERO,
        flags: 0,
        born_frame: 0,
    };
    let h = ctx.body.create_laser(init);
    push(task, pack_laser_handle(h))
}

/// `lz_speed(lz, speed, start_len)`（801）。
fn sys_laser_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let start_len = pop(task)?;
    let speed = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body
        .laser_set_speed(h, Fx::from_raw(speed), Fx::from_raw(start_len));
    Ok(())
}

/// `lz_start(lz, s)`（802）。
fn sys_laser_start(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let s = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_set_start(h, Fx::from_raw(s));
    Ok(())
}

/// `lz_omega(lz, a)`（803）：`a` 取低 16 位**按位回绕**为 `i16`（BAM 语义——反向扫射编码后
/// 原始值 > 32767，位穿透才能保持反向），不钳位、不计数。
fn sys_laser_omega(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_set_omega(h, bam(a).raw() as i16);
    Ok(())
}

/// `lz_rotate(lz, a)`（804）：一次性转 `a`（回绕加）。
fn sys_laser_rotate(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_rotate(h, bam(a));
    Ok(())
}

/// `lz_aim(lz, off)`（805）：指向自机 0 的角度 + `off`。
fn sys_laser_aim(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let off = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_aim(h, bam(off));
    Ok(())
}

/// `lz_anchor(lz, enemy, ox, oy)`（806）。`enemy = -1` → `EnemyHandle::NULL`（解除）；
/// 其它敌号一律经 [`resolve_enemy_handle`] 解析，命中则用池里的**完整 u16 代际**重建句柄
/// （I-3）；解析失败 → 不挂靠 + 计一次 `contract_viol`（控制方裁定 ②）。
fn sys_laser_anchor(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let oy = pop(task)?;
    let ox = pop(task)?;
    let enemy = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    let e = if enemy == -1 {
        EnemyHandle::NULL
    } else {
        match resolve_enemy_handle(enemy, ctx) {
            Some(idx) => EnemyHandle {
                index: idx as u16,
                generation: ctx.body.enemies.generation[idx],
            },
            None => {
                // 失效敌号（死了/代际不符/越界）：no-op + 计数，不落到世界侧。
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_STALE_HANDLE;
                return Ok(());
            }
        }
    };
    ctx.body
        .laser_anchor(h, e, Fx::from_raw(ox), Fx::from_raw(oy));
    Ok(())
}

/// `lz_origin(lz, x, y)`（807）：直接设原点并解除挂靠。
fn sys_laser_origin(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let y = pop(task)?;
    let x = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_origin(h, Fx::from_raw(x), Fx::from_raw(y));
    Ok(())
}

/// `lz_cancel(lz)`（808）。
fn sys_laser_cancel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_cancel(h);
    Ok(())
}

/// `lz_alive(lz)`（809）：只读，不计数；押 0/1。
fn sys_laser_alive(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let lz = pop(task)?;
    let alive = resolve_laser_handle(lz, ctx).is_some();
    push(task, if alive { 1 } else { 0 })
}

/// 激光几何读口（810–814）共用：只读、不计数；失效句柄押 0（同 `enemy_x` 读族口径）。
fn sys_laser_read(task: &mut Task, ctx: &mut VmCtx, id: u16) -> Result<(), u8> {
    let lz = pop(task)?;
    let v = match resolve_laser_handle(lz, ctx) {
        None => 0,
        Some(h) => {
            let (l, i) = (&ctx.body.lasers, h.index as usize);
            match id {
                SYS_LASER_X => l.ox[i].raw(),
                SYS_LASER_Y => l.oy[i].raw(),
                SYS_LASER_ANGLE => i32::from(l.angle[i].raw()),
                SYS_LASER_NEAR => l.start[i].raw(),
                _ => l.end[i].raw(),
            }
        }
    };
    push(task, v)
}

/// `n<=0` → 押 0、不消耗世界 RNG 流（拍板：n==0 既定钉死，n<0 视同"无合法范围"同律扩展、
/// 同样不消耗——一次坏参不该让世界 RNG 流分叉）。
fn sys_rand_range(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let n = pop(task)?;
    let v = if n <= 0 {
        0
    } else {
        ctx.body.rng.rand_range(n as u32) as i32
    };
    push(task, v)
}

/// 丙方案 create_bullet（SYS 200）：8 参逆序弹出；appearance 越界/xform 区间越界 → Fault；
/// task_script ≥0 时脚本号必须在册（否则 Fault，同 `OP_SPAWN` 口径）——**先查后建**：
/// 一切校验在任何世界写之前完成，避免"弹已建、任务绑定失败"的半成品状态。
/// 创建失败（appearance/xform 校验过关后，池满或 xform 内容仍被 `create_bullet_with_xform`
/// 拒绝）→ 押 -1，不派生任务。
fn sys_create_bullet(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let task_script = pop(task)?;
    let xform_cnt = pop(task)?;
    let xform_off = pop(task)?;
    let angle_raw = pop(task)?;
    let speed_raw = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let appearance = pop(task)?;

    let Some(cfg) = ctx.tables.appearances.get(appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    // 空格格（图集该格没有图）→ 拒。放行会造出"有判定但画面上什么都没有"的隐形弹，
    // 而金向量/冒烟都抓不到它（校验和不关心贴图内容）。先验后建：此处尚未写世界。
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }

    // xform 区间越界（LOCALS 边界）→ Fault；内容层面的坏 xform（>16 槽/未知 op/...）留给
    // `create_bullet_with_xform` 自己的 P4-b 处置（NULL + BAD_ARGS，见下方"押 -1"分支）。
    let mut xform_buf = [XformSlot::default(); 16];
    let xform_len: usize = if xform_cnt == 0 {
        0
    } else {
        if xform_cnt < 0 || xform_cnt as usize > 16 || xform_off < 0 {
            return Err(FAULT_BAD_OP);
        }
        let cnt = xform_cnt as usize;
        let off = xform_off as usize;
        let Some(end) = off.checked_add(cnt * 3) else {
            return Err(FAULT_BAD_OP);
        };
        if end > LOCALS {
            return Err(FAULT_BAD_OP);
        }
        for (k, slot) in xform_buf.iter_mut().take(cnt).enumerate() {
            let base = off + k * 3;
            let word0 = task.locals[base] as u32;
            *slot = XformSlot {
                wait: (word0 >> 16) as u16,
                op: ((word0 >> 8) & 0xFF) as u8,
                _pad: 0,
                args: [task.locals[base + 1], task.locals[base + 2]],
            };
        }
        cnt
    };

    // task_script 号必须在册（先查，不建弹）——空指针式坏号同 OP_SPAWN 口径复用 FAULT_BAD_OP。
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async
            || ctx
                .ecl
                .param_types(sub)
                .is_none_or(|params| !params.is_empty())
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else {
        None
    };

    let angle = bam(angle_raw);
    let speed = Fx::from_raw(speed_raw);
    let (vx, vy) = polar_to_vec(speed, angle);
    let init = BulletInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
        vx,
        vy,
        speed,
        angle,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: cfg.sprite,
        radius: cfg.radius,
        delay: 0,
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: crate::xform::XFORM_NONE,
        xform_wait: 0,
        xform_next: 0,
        born_frame: ctx.body.frame,
    };

    let handle = if xform_len == 0 {
        ctx.body.create_bullet(init)
    } else {
        ctx.body
            .create_bullet_with_xform(init, &xform_buf[..xform_len])
    };

    if handle == BulletHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)?;

    if let Some(task_sub) = task_sub {
        // entry 已在上面校验过在册；池满 → 静默计数（P4-a），弹已建、句柄已押，不 Fault。
        let pc0 = ctx
            .ecl
            .sub_meta(task_sub)
            .expect("已在上面校验过")
            .code_entry();
        let owner = (OWNER_BULLET, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        if ctx
            .tasks
            .spawn(task_sub, pc0, owner, parent, ctx.frame)
            .is_none()
        {
            ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
        }
    }
    Ok(())
}

/// 批量环（SYS 201）：9 参逆序弹出，无 xform（性能语义原语，dumb 弹批量铺环）。
/// appearance 越界 → Fault；池满/坏轴由 `create_bullets_batch` 自身 P4 处置（压实发数，
/// 可能为 0——not 视为 Fault，与单发 create_bullet 的"NULL→-1"口径不同因为返回语义本就是数量）。
fn sys_create_bullets_batch(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let speed_step = pop(task)?;
    let speed0 = pop(task)?;
    let n_speed = pop(task)?;
    let angle_step = pop(task)?;
    let angle0 = pop(task)?;
    let n_angle = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let appearance = pop(task)?;

    let Some(cfg) = ctx.tables.appearances.get(appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    // 空格格（图集该格没有图）→ 拒。放行会造出"有判定但画面上什么都没有"的隐形弹，
    // 而金向量/冒烟都抓不到它（校验和不关心贴图内容）。先验后建：此处尚未写世界。
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }

    let init = BulletInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle::ZERO,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: cfg.sprite,
        radius: cfg.radius,
        delay: 0,
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: crate::xform::XFORM_NONE,
        xform_wait: 0,
        xform_next: 0,
        born_frame: ctx.body.frame,
    };

    let n = ctx.body.create_bullets_batch(
        init,
        &[],
        n_angle.max(0) as u16,
        bam(angle0),
        angle_step as i16,
        n_speed.max(0) as u16,
        Fx::from_raw(speed0),
        Fx::from_raw(speed_step),
    );
    push(task, n as i32)
}

/// 敌人创建（SYS 210；A5 乙案，append-only：旧 5 参前缀不动，尾追 sprite/task）：
/// 7 参逆序弹出。半径/受击盒用固定默认值（同 `world::test_support::spawn_enemy` 惯例）；
/// `task` 号先验后建（镜像 `sys_create_bullet` 的 task 路径：坏号/非 0 参 Async → Fault，
/// 零副作用，敌未建）；`main_task` 在敌句柄产出**之后**回填（任务的 owner 三元组需要敌
/// index/gen，敌必须先于任务存在）；`death_script` 仍恒 0（脚本面缺口留 follow-ups）。
fn sys_spawn_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    // boss 换段刀 spec §4.2：压栈序 `x,y,hp,drop,score,sprite,task_script, arg0..arg(n-1), argc`。
    // 门禁全部先于建敌（任一门不过都零副作用）；argc/栈深口径同 `OP_SPAWN`。
    let argc = usize::try_from(pop(task)?)
        .ok()
        .filter(|&n| n <= LOCALS)
        .ok_or(FAULT_STACK)?;
    if (task.sp as usize) < argc + 7 {
        return Err(FAULT_STACK);
    }
    let mut args = [0i32; LOCALS];
    for k in (0..argc).rev() {
        args[k] = pop(task)?;
    }
    let task_script = pop(task)?;
    let sprite = pop(task)?;
    let score = pop(task)?;
    let drop_table = pop(task)?;
    let hp = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;

    // task 号先验后建（镜像 sys_create_bullet：坏号 FAULT_BAD_OP，敌未建）：在册 + Async +
    // 形参个数 == argc；none 不许带参。
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async || ctx.ecl.param_types(sub).is_none_or(|p| p.len() != argc)
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else if argc > 0 {
        return Err(FAULT_BAD_OP);
    } else {
        None
    };

    // 掉落表号在**生成时**展开成逐类型计数（此前存表号、死时才查表）。
    // P4-b：越界表号 → 视同空表 + 计数（原检查在 `settle::damage_enemy`，随状态前移）。
    // `contract_viol` 与 `last_status` **两样都写**——邻居的每条 P4-b 都是这个口径
    // （`world::add_enemy_drop` / `world::move_enemy_to` / `sys_spell_begin`），搬到 syscall
    // 层之后不该变成异类。`last_status` 进校验和，故这一处会改世界状态（金向量两侧表号
    // 恒不越界，压不到这条路径，实测逐字节不变）。
    let (drop_count, table_ok) = crate::tables::drop_counts(ctx.tables, drop_table as u16);
    if !table_ok {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
    }

    let init = EnemyInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        dx: Fx::ZERO,
        dy: Fx::ZERO,
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
        hp,
        hp_max: hp,
        radius: Fx::from_int(12),
        hurtbox: Fx::from_int(16),
        invuln: 0,
        hit_flash: 0,
        flags: 0,
        sprite: sprite as u16,
        anm_state: 0,
        anm_state_frame: ctx.body.frame,
        main_task: 0, // 任务 spawn 后回填（敌句柄先于任务存在）
        death_script: 0,
        drop_count,
        score: score as u16,
    };
    let handle = ctx.body.create_enemy(init);
    if handle == EnemyHandle::NULL {
        return push(task, -1);
    }
    push(task, crate::enemy::pack_handle(handle))?;

    if let Some(sub) = task_sub {
        // entry 已在上面校验过在册；池满 → 静默计数（P4-a），敌已建、句柄已押，不 Fault。
        let pc0 = ctx.ecl.sub_meta(sub).expect("已在上面校验过").code_entry();
        let owner = (OWNER_ENEMY, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        match ctx.tasks.spawn(sub, pc0, owner, parent, ctx.frame) {
            Some(slot) => {
                ctx.tasks.write_args(slot, &args[..argc]);
                ctx.body.enemies.main_task[handle.index as usize] = slot as u32 + 1;
            }
            None => {
                ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                    ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
            }
        }
    }
    Ok(())
}

/// 道具掉落（SYS 220）：3 参逆序弹出。坏类型 → `drop_item` 自身 P4-b 处置（NULL + BAD_ARGS
/// 计数，不 Fault——世界既有 write API 契约，syscall 层不重复判定）。
fn sys_drop_item(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let item_type = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let handle = ctx.body.drop_item(
        Fx::from_raw(x_raw),
        Fx::from_raw(y_raw),
        item_type as u8,
        ctx.tables,
    );
    if handle == crate::items::ItemHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)
}

/// 敌人限时缓动位移（SYS 400）：self owner 必须是 ENEMY（否则 Fault，misuse 策略）；
/// 4 参逆序弹出：`easing, y, x, dur`。无返回值。
fn sys_move_enemy_to(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19：越界 dur/easing → P4-b 整条 no-op
    };
    ctx.body
        .move_enemy_to(h, Fx::from_raw(x_raw), Fx::from_raw(y_raw), dur, easing);
    Ok(())
}

/// 运动动词族的 `dur`/`easing` **参数收窄**（D19，2026-09-03 裁定：收窄成拒收）。
///
/// 此前五处一律裸 `as u16` / `as u8`，制造了两个坏路径：
/// - `easing = 256` → `256 as u8 == 0` → **静默变成 Linear**，绕过世界层 `easing >= 8` 的
///   判据（那条判据收到的已经是截断后的值）；写 `easing = 264` 反而落 8、被正确拒掉
///   ——**能不能拒取决于越界值模 256 落在哪里，毫无规律**。
/// - `dur = -1` → `-1 as u16 == 65535` → 一条本该报错的笔误变成"缓动 18 分钟"。
///
/// 现在两个都走 `try_from`：任一失败即 **P4-b**（`contract_viol` +1 + `last_status =
/// BAD_ARGS` + **整条 no-op**），与既有的 `easing >= 8` 是同一条腿，判据顺势前移到收窄这步。
/// 不 Fault——参数笔误属"调用方违约"，确定性安全结果比杀任务有用。
///
/// 返回 `None` 即调用方应当整条 no-op。
fn narrow_dur_easing(ctx: &mut VmCtx, dur: i32, easing: i32) -> Option<(u16, u8)> {
    match (u16::try_from(dur), u8::try_from(easing)) {
        (Ok(d), Ok(e)) => Some((d, e)),
        _ => {
            ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
            ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
            None
        }
    }
}

/// `SYS_MOVE_VEL`（410）：逆序弹出 `easing, speed, angle, dur`。
fn sys_move_vel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let speed = pop(task)?;
    let angle = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body.set_enemy_vel_polar(
        h,
        crate::math::Angle(angle as u16),
        Fx::from_raw(speed),
        dur,
        easing,
    );
    Ok(())
}

/// `SYS_MOVE_VEL_XY`（411）：逆序弹出 `easing, vy, vx, dur`。
fn sys_move_vel_xy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let vy = pop(task)?;
    let vx = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body
        .set_enemy_vel_cart(h, Fx::from_raw(vx), Fx::from_raw(vy), dur, easing);
    Ok(())
}

/// `SYS_MOVE_ANGLE`（420）：逆序弹出 `easing, angle, dur`。
fn sys_move_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let angle = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body
        .set_enemy_angle(h, crate::math::Angle(angle as u16), dur, easing);
    Ok(())
}

/// `SYS_MOVE_SPEED`（421）：逆序弹出 `easing, speed, dur`。
fn sys_move_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let speed = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body
        .set_enemy_speed(h, Fx::from_raw(speed), dur, easing);
    Ok(())
}

/// boss 公告板整槽写（SYS 730）：`enemy` 字段取自 self owner（非 ENEMY → `EnemyHandle::NULL`，
/// 不 Fault——见模块文档"误用策略"boss_set 例外）；6 参逆序弹出：
/// `active, phase_left, timer_frames, spell_id, hp_ratio, slot`。无返回值。
fn sys_boss_set(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let active = pop(task)?;
    let phase_left = pop(task)?;
    let timer_frames = pop(task)?;
    let spell_id = pop(task)?;
    let hp_ratio = pop(task)?;
    let slot = pop(task)?;

    let enemy = if task.owner_kind == OWNER_ENEMY {
        EnemyHandle {
            index: task.owner_index,
            generation: task.owner_gen,
        }
    } else {
        EnemyHandle::NULL
    };

    ctx.body.boss_set(
        slot as u8,
        BossUiSlot {
            enemy,
            hp_ratio: Fx::from_raw(hp_ratio),
            spell_id: spell_id as u16,
            timer_frames: timer_frames as u16,
            phase_left: phase_left as u8,
            active: active as u8,
        },
    );
    Ok(())
}

/// `SYS_EMIT_REQ`（720）：通道 B 推送。id 收窄 P4-b——栈值超出 `0..=65535` →
/// no-op + `contract_viol` + `BAD_ARGS`，**不 Fault**（作者违约 → 确定性安全结果）；
/// 值域内转交 `WorldBody::emit_req`（满缓冲处置 TRUNCATED + 计数在那边）。
fn sys_emit_req(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a5 = pop(task)?;
    let a4 = pop(task)?;
    let a3 = pop(task)?;
    let a2 = pop(task)?;
    let a1 = pop(task)?;
    let a0 = pop(task)?;
    let id = pop(task)?;
    if !(0..=u16::MAX as i32).contains(&id) {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }
    ctx.body.emit_req(id as u16, [a0, a1, a2, a3, a4, a5]);
    Ok(())
}

/// 敌人表现状态号（[`SYS_SET_ANM_STATE`]=430）：1 参逆序弹出；self-only（非敌 → Fault）。
/// 悬垂 owner 句柄由世界层 `set_anm_state` 计 P4-b（no-op + `STALE_HANDLE`）。
fn sys_set_anm_state(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let state = pop(task)?;
    ctx.body.set_anm_state(h, state as u16);
    Ok(())
}

/// 定位一次性演出（[`SYS_FX_AT`]=721）：4 参逆序弹出，owner 无限制。布局钉死为
/// `REQ_FX_AT, [x raw, y raw, kind, param, 0, 0]`；缓冲满走 D12（丢弃 + `reqs_dropped`）。
fn sys_fx_at(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let param = pop(task)?;
    let kind = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    ctx.body
        .emit_req(crate::consts::REQ_FX_AT, [x_raw, y_raw, kind, param, 0, 0]);
    Ok(())
}

/// 依附一次性演出（[`SYS_FX_ON`]=722）：2 参逆序弹出；self-only（非敌 → Fault）。
/// 布局钉死为 `REQ_FX_ATTACHED, [index, gen, kind, param, 0, 0]`——**裸 index/gen 两位**，
/// 不用 `crate::enemy::pack_handle` 的打包形态（那是脚本值域内的敌号；请求载荷给壳侧，两位分开
/// 免得壳侧再拆包）。
fn sys_fx_on(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let param = pop(task)?;
    let kind = pop(task)?;
    ctx.body.emit_req(
        crate::consts::REQ_FX_ATTACHED,
        [h.index as i32, h.generation as i32, kind, param, 0, 0],
    );
    Ok(())
}

/// 关卡结束（[`SYS_STAGE_CLEAR`]=723）：1 参弹出，发 `EVT_STAGE_CLEARED{data0 = stage}`。
/// 世界侧只记事实，不改任何状态；让出一帧由表层 codegen 追发的 `WAIT` 负责。
fn sys_stage_clear(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let stage = pop(task)?;
    ctx.body.push_event(crate::events::Event {
        kind: crate::events::EVT_STAGE_CLEARED,
        data: [stage, 0],
        ..Default::default()
    });
    Ok(())
}

/// 符卡宣言（`SYS_SPELL_BEGIN`=740；符卡机构 spec 2026-07-24 §5）：7 参逆序弹出；
/// `self_enemy_handle`（非敌 misuse → Fault，同 `move_enemy_to` 误用策略）。
///
/// **四条控制器补充决策**（复审 T1-m3/T1-m1/Task2 复审落地，spec 未写全，本刀新增，
/// syscall 层专属——世界侧 `spell_begin_internal` 仍宽松接受负 threshold/双绑定供白盒测
/// 直调）：
/// - **拒负 threshold**：血线语义非负，`threshold < 0` → P4-b no-op + 计数，不建。
/// - **防双绑定**：该 boss 已绑到**另一** active 槽 → P4-b no-op + 计数（单卡/boss 是
///   设计用法，多卡序靠脚本顺序 begin 下一张，不允许并行两槽绑同一 boss）。
/// - **拒负 bonus0**（Task 2 复审 Important #2，spec §5 P4-b 拒收条件）：分数语义非负，
///   `bonus0 < 0` → P4-b no-op + 计数（必须在 `as u32` 转型前拒，否则变 `u32::MAX`）。
/// - **拒非正 time_limit**（Task 2 复审 Important #3，spec §5 P4-b 拒收条件）：
///   `time_limit <= 0` → P4-b no-op + 计数（必须在 `as u16` 转型前拒，否则负值抹符号
///   变成巨大正数，绕过 `spell_begin_internal` 里"`time_limit == 0`"那条守卫）。
///
/// `pattern`（`SubRef`，负值=none）**先查后建**（同 `sys_create_bullet`/`OP_SPAWN` 坏号
/// 口径）：越界/非 0 参 Async 号 → Fault，零副作用（`spell_begin_internal` 尚未调用）。
/// `spell_begin_internal` 返 `true` 且 `pattern` 非 none → 照 fire task-spawn 样板 spawn
/// 模式任务（owner=boss，`spell_bound = slot+1` + `spell_epoch` 读回本槽刚铸出的代际戳——
/// "生"，spec §2.1 + ABA 修复复审 Task 2）；返 `false` → 不 spawn（P4-b 已在 internal 计数）。
fn sys_spell_begin(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let threshold = pop(task)?;
    let flags = pop(task)?;
    let bonus0 = pop(task)?;
    let time_limit = pop(task)?;
    let pattern_ref = pop(task)?;
    let spell_id = pop(task)?;
    let slot = pop(task)?;
    let h = self_enemy_handle(task)?;

    if threshold < 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // 复审 Task 2 补充决策（Important #2，spec §5 P4-b 拒收条件）：`bonus0 < 0` 必须在
    // `bonus0 as u32` 转型**之前**拒收——否则 `-1i32 as u32` 变 `u32::MAX`，等于凭空发
    // ~42.9 亿分。血线/分数语义都非负，同 `threshold < 0` 口径处置：no-op + 计数，不建。
    if bonus0 < 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // 复审 Task 2 补充决策（Important #3，spec §5 P4-b 拒收条件）：`time_limit <= 0` 必须在
    // `time_limit as u16` 转型**之前**拒收——`spell_begin_internal` 只查 `time_limit == 0`，
    // 而负值经 `as u16` 抹符号会变成一个巨大的正数（如 `-1 → 65535`），从而绕过那条守卫、
    // 带着离谱的时限成功 begin。血线语义要求正时限，同上口径处置。
    if time_limit <= 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    let already_bound_elsewhere = (0..crate::boss::MAX_BOSSES).any(|s| {
        ctx.body.spells[s].active != 0
            && ctx.body.spells[s].boss_index == h.index
            && ctx.body.spells[s].boss_gen == h.generation
    });
    if already_bound_elsewhere {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // pattern 号先查后建：坏号 Fault、none（负值）跳过 spawn，均在 internal 调用之前定案。
    let pattern_sub: Option<SubId> = if pattern_ref >= 0 {
        let raw = u16::try_from(pattern_ref).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async
            || ctx
                .ecl
                .param_types(sub)
                .is_none_or(|params| !params.is_empty())
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else {
        None
    };

    let slot_idx = slot as usize; // 负/越界值 wrapping 成巨大 usize，internal 的越界门自然挡
    let began = ctx.body.spell_begin_internal(
        slot_idx,
        h,
        spell_id as u16,
        time_limit as u16,
        bonus0 as u32,
        flags as u8,
        threshold,
    );
    if began && let Some(pattern_sub) = pattern_sub {
        // 照 fire 的 task-spawn 样板（sys_create_bullet）：entry 已在上面校验过在册。
        let pc0 = ctx
            .ecl
            .sub_meta(pattern_sub)
            .expect("已在上面校验过")
            .code_entry();
        let owner = (OWNER_ENEMY, h.index, h.generation);
        let parent = ctx.self_index + 1;
        match ctx.tasks.spawn(pattern_sub, pc0, owner, parent, ctx.frame) {
            Some(idx) => {
                // "生"：模式任务显式绑定本槽（spec §2.1），与 OP_SPAWN 的"继承"路径
                // 不同——这是模式树的根，绑定值凭空而来，不是继承自父任务。
                ctx.tasks.slots[idx as usize].spell_bound = (slot_idx + 1) as u8;
                // ABA 修复（复审 Task 2，Critical）：同批写入本槽刚铸出的代际戳（begin 内
                // 已在 `spell_begin_internal` 里推进过 `spell_seq[slot]` 并戳进
                // `spells[slot].epoch`，此处读回）——模式任务据此锁定"自己是这一代"，
                // 槽将来被复用给别的卡时，相位 2 调度门禁靠这个戳把它挡在门外。
                ctx.tasks.slots[idx as usize].spell_epoch = ctx.body.spells[slot_idx].epoch;
            }
            None => {
                ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                    ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
            }
        }
    }
    Ok(())
}

/// 符卡逃生舱口（`SYS_SPELL_END`=741；符卡机构 spec 2026-07-24 §5）：无参；
/// `self_enemy_handle`（非敌 misuse → Fault）；转交 `spell_end_by_owner`（无绑定 →
/// no-op，重复调用安全，见该 API 文档）。
fn sys_spell_end(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    ctx.body.spell_end_by_owner(h);
    Ok(())
}

/// 瞄准角查询（SYS 120）：0 参；self 位置（owner 未知/STAGE→原点）朝向**目标自机**的
/// `atan2`，押回 BAM raw（不消 RNG、不改世界，纯读）。
///
/// 目标自机走引擎唯一的"瞄谁"口径 [`crate::world::WorldBody::aim_target`]（F8 统一，
/// 2026-09-03）：从 self 位置看过去最近的可瞄自机，一个可瞄的都没有则回退 `players[0]`
/// 的最后坐标——查询必须产出一个角度。
/// 单人局（`players[1]` 恒 ABSENT）下这与旧的"恒 `players[0]`"逐位等同——改的是 co-op：
/// P1 已 game over 而 P2 还活着时，不再朝着尸体坐标算角。
fn sys_aim_player_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (sx, sy) = self_pos(task, ctx);
    let p = ctx.body.aim_target(sx, sy);
    let px = ctx.body.players[p].x;
    let py = ctx.body.players[p].y;
    let angle = crate::math::cordic::atan2(py - sy, px - sx);
    push(task, angle.raw() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::image::{EclImage, EntryInit, SubInit, SubKind, test_image};
    use crate::ecl::shooter::{SH_ABS_OFFSET, SH_AIMED, SH_RING, SHOOTERS_PER_TASK, ShooterSlot};
    use crate::ecl::task::{OWNER_ENEMY, OWNER_STAGE, Task};
    use crate::step::World;
    use crate::tables::TABLES_V0;

    // 外观表行号（颜色轴刀 T4 起 ② 段清空、旧的 `APPEARANCE_*` 引擎常量退场——弹型名
    // 归内容包的 `.ecl` const，引擎侧测试直接写行号）。id = 弹型 × color_stride(16) + 色号：
    // 0 = 0 号形第 0 色，1 = 0 号形第 1 色，2 = 0 号形第 2 色。三者只用于"随便挑一个
    // 合法格"，与具体形/色语义无关。
    const ROW_A: i32 = 0;
    const ROW_B: i32 = 1;
    const ROW_C: i32 = 2;

    /// 冻结号表（号, 名, 期望族号）——逐条照 spec §4 的表；改动本表 = 改冻结面 = 过评审。
    /// 提成助手供两条测试共用：结构判据（本表自身的性质）与白名单判据
    /// （[`syscall_implemented`] 必须与本表**逐条等同**）。
    fn frozen_table() -> &'static [(u16, &'static str, u16)] {
        &[
            (SYS_FRAME, "frame", 0),
            (SYS_PLAYER_X, "player_x", 0),
            (SYS_PLAYER_Y, "player_y", 0),
            (SYS_SELF_X, "self_x", 0),
            (SYS_SELF_Y, "self_y", 0),
            (SYS_SELF_VX, "self_vx", 0),
            (SYS_SELF_VY, "self_vy", 0),
            (SYS_SELF_SPEED, "self_speed", 0),
            (SYS_SELF_ANGLE, "self_angle", 0),
            (SYS_SELF_HP, "self_hp", 0),
            (SYS_SELF_HP_MAX, "self_hp_max", 0),
            (SYS_SELF_AGE, "self_age", 0),
            (SYS_SELF_ENEMY, "self_enemy", 0),
            (SYS_ENEMY_HP, "enemy_hp", 1),
            (SYS_ENEMY_X, "enemy_x", 1),
            (SYS_ENEMY_Y, "enemy_y", 1),
            (SYS_ENEMY_ALIVE, "enemy_alive", 1),
            (SYS_NEAREST_ENEMY, "nearest_enemy", 1),
            (SYS_AIM_PLAYER_ANGLE, "aim_player_angle", 1),
            (SYS_SPELL_TIMER, "spell_timer", 1),
            (SYS_SPELL_RESULT, "spell_result", 1),
            (SYS_ATAN2, "atan2", 1),
            (SYS_DIST, "dist", 1),
            (SYS_RAND_RANGE, "rand_range", 1),
            (SYS_CREATE_BULLET, "create_bullet", 2),
            (SYS_CREATE_BULLETS_BATCH, "create_bullets_batch", 2),
            (SYS_SPAWN_ENEMY, "spawn_enemy", 2),
            (SYS_DROP_ITEM, "drop_item", 2),
            (SYS_SET_BULLET_SPEED, "set_bullet_speed", 3),
            (SYS_SET_BULLET_ANGLE, "set_bullet_angle", 3),
            (SYS_TURN_BULLET, "turn_bullet", 3),
            (SYS_SET_BULLET_VEL, "set_bullet_vel", 3),
            (SYS_SET_BULLET_ANG_VEL, "set_bullet_ang_vel", 3),
            (SYS_SET_BULLET_ACCEL, "set_bullet_accel", 3),
            (SYS_SET_BULLET_GRAVITY, "set_bullet_gravity", 3),
            (SYS_STOP_BULLET_FX, "stop_bullet_fx", 3),
            (SYS_AIM_BULLET_AT_PLAYER, "aim_bullet_at_player", 3),
            (SYS_MOVE_ENEMY_TO, "move_enemy_to", 4),
            (SYS_MOVE_VEL, "move_vel", 4),
            (SYS_MOVE_VEL_XY, "move_vel_xy", 4),
            (SYS_MOVE_ANGLE, "move_angle", 4),
            (SYS_MOVE_SPEED, "move_speed", 4),
            (SYS_SET_ANM_STATE, "set_anm_state", 4),
            (SYS_SET_INVULN, "set_invuln", 4),
            (SYS_SET_HITBOX, "set_hitbox", 4),
            (SYS_SET_HURTBOX, "set_hurtbox", 4),
            (SYS_SET_ENEMY_FLAG, "set_enemy_flag", 4),
            (SYS_ADD_SCORE, "add_score", 5),
            (SYS_ADD_LIVES, "add_lives", 5),
            (SYS_ADD_BOMBS, "add_bombs", 5),
            (SYS_ADD_POWER, "add_power", 5),
            (SYS_DROP_CLEAR, "drop_clear", 5),
            (SYS_DROP_ADD, "drop_add", 5),
            (SYS_DROP_ITEMS, "drop_items", 5),
            (SYS_DIE, "die", 5),
            (SYS_KILL_ALL_ENEMIES, "kill_all_enemies", 5),
            (SYS_CLEAR_BULLETS, "clear_bullets", 5),
            (SYS_CLEAR_BULLETS_AT, "clear_bullets_at", 5),
            (SYS_BGM, "bgm", 5),
            (SYS_BG, "bg", 5),
            (SYS_BG_PHASE, "bg_phase", 5),
            (SYS_TIME_STOP_PLAYER, "time_stop_player", 5),
            (SYS_SH_RESET, "sh_reset", 6),
            (SYS_SH_SPRITE, "sh_sprite", 6),
            (SYS_SH_OFFSET, "sh_offset", 6),
            (SYS_SH_OFFSET_ABS, "sh_offset_abs", 6),
            (SYS_SH_OFFSET_RAD, "sh_offset_rad", 6),
            (SYS_SH_DIST, "sh_dist", 6),
            (SYS_SH_ANGLE, "sh_angle", 6),
            (SYS_SH_SPEED, "sh_speed", 6),
            (SYS_SH_COUNT, "sh_count", 6),
            (SYS_SH_AIM, "sh_aim", 6),
            (SYS_SH_RING, "sh_ring", 6),
            (SYS_SH_XFORM, "sh_xform", 6),
            (SYS_SH_TASK, "sh_task", 6),
            (SYS_SH_REQ, "sh_req", 6),
            (SYS_SH_FIRE, "sh_fire", 6),
            (SYS_GET_VAR, "get_var", 7),
            (SYS_SET_VAR, "set_var", 7),
            (SYS_PULSE_SIGNAL, "pulse_signal", 7),
            (SYS_EMIT_REQ, "emit_req", 7),
            (SYS_FX_AT, "fx_at", 7),
            (SYS_FX_ON, "fx_on", 7),
            (SYS_STAGE_CLEAR, "stage_clear", 7),
            (SYS_BOSS_SET, "boss_set", 7),
            (SYS_SPELL_BEGIN, "spell_begin", 7),
            (SYS_SPELL_END, "spell_end", 7),
            (SYS_LASER_CREATE, "laser", 8),
            (SYS_LASER_SPEED, "lz_speed", 8),
            (SYS_LASER_START, "lz_start", 8),
            (SYS_LASER_OMEGA, "lz_omega", 8),
            (SYS_LASER_ROTATE, "lz_rotate", 8),
            (SYS_LASER_AIM, "lz_aim", 8),
            (SYS_LASER_ANCHOR, "lz_anchor", 8),
            (SYS_LASER_ORIGIN, "lz_origin", 8),
            (SYS_LASER_CANCEL, "lz_cancel", 8),
            (SYS_LASER_ALIVE, "lz_alive", 8),
            (SYS_LASER_X, "lz_x", 8),
            (SYS_LASER_Y, "lz_y", 8),
            (SYS_LASER_ANGLE, "lz_angle", 8),
            (SYS_LASER_NEAR, "lz_near", 8),
            (SYS_LASER_FAR, "lz_far", 8),
        ]
    }

    /// 【本刀的主判据】号表族结构：102 条、无重号、每条落在其声明族的百位区间内
    /// （原 74 条 + 自机能力刀 `513`/`560` = 76；表现契约 v2 再加 `430 set_anm_state`/`721 fx_at`/`722 fx_on` = 79；壳子刀加 `723 stage_clear` = 80；
    /// 激光池刀再加 8xx 十条 = 97；激光读口五条 810–814 = 102）。
    ///
    /// 这一刀是大规模机械重排，判别力要求与常规刀不同——不是"新行为对不对"，而是
    /// "**有没有搬错、搬漏、搬重**"。故判据是号表自身的结构性质，不是某条 syscall 的行为。
    ///
    /// # 判别力边界（终审实测，2026-07-31——别高估这条测试）
    ///
    /// **抓不到「族内互换」。** [`frozen_table`] 是**符号引用**常量的（`(SYS_ATAN2, "atan2",
    /// 1)`），所以把 `SYS_ATAN2` 与 `SYS_DIST` 的值对调（140 ↔ 141）之后，本测试的三条
    /// 断言**全部照过**：仍是 74 条、仍两两不等、仍都落在 `1xx`。实测把这对值对调后
    /// **全仓 947 条测试无一转红**——这不是覆盖漏洞，因为号本身是任意的，族内换值在行为上
    /// 确实是 no-op（调用方一律符号引用，`ecl-meta.json` 也不含号）。但上面那句"有没有
    /// 搬错"要按字面读会读出过强的承诺：**「值错、族对」这一格本测试是瞎的**。
    /// 那一格**全仓唯一**的网是 [`ecl_ops_doc_syscall_numbers_match_the_constants`]——
    /// 它拿 `docs/ecl-ops.md`（号表的权威呈现面）里的**字面数字**双向对常量，
    /// 别把它当成只防文档漂移的东西而删掉。
    ///
    /// **「搬重」比本测试更早被抓住——是编译期错误。** 把 `SYS_DIST` 改成与 `SYS_ATAN2`
    /// 同值，rustc 在**两处**报 `unreachable pattern`：[`syscall_implemented`] 的白名单
    /// `matches!` 与 [`dispatch`] 的 `match`（两处都是 `const` 模式匹配）。CI 的
    /// `-D warnings` 下这是硬失败，压根编译不过来跑测试。故 (b) 那条唯一性断言实际是
    /// **第二道网**，价值在于把这条性质写成可读的意图，而不是它先发现问题。
    #[test]
    fn syscall_table_is_hundred_partitioned_and_unique() {
        let table = frozen_table();

        assert_eq!(
            table.len(),
            102,
            "74 + 自机能力刀两条（513/560）+ 表现契约 v2 三条（430/721/722）+ 壳子刀 723 − 玩法刀退役 513 \
             + boss 换段刀八条（026/131/440/441/442/443/531/541）= 87；激光池刀 8xx 十条 = 97；激光读口 810–814 = 102：增改需同步这个数"
        );

        // (a) 族归属：搬错族立刻红
        for &(num, name, fam) in table {
            assert_eq!(
                num / 100,
                fam,
                "{name}(={num}) 应落在 {fam}xx 族，实得 {}xx",
                num / 100
            );
        }

        // (b) 两两不等：搬重立刻红（O(n²) 但 n=76，测试里无所谓）
        for (i, &(a, na, _)) in table.iter().enumerate() {
            for &(b, nb, _) in &table[i + 1..] {
                assert_ne!(a, b, "{na} 与 {nb} 撞号（都是 {a}）");
            }
        }

        // (c) 与 op 号空间错开：op 是 u8、现最大 60（OP_SPAWN_PATTERN）。这条只对**非
        //     0xx 族**成立——1xx..7xx 全部 >= 100，与 op 号空间永久不相交；0xx 族的号
        //     本来就 < 100（000-032），与 op 号空间的重叠是既有状态，本刀不承诺解决
        //     （spec §3 "syscall 全部推到 100 以上就与 op 号错开"这句对 0xx 族不成立，
        //     已在设计评审收窄，见 task-1-report.md）。
        for &(num, name, fam) in table {
            if fam != 0 {
                assert!(
                    num >= 100,
                    "{name}(={num}) 应与 op 号空间(u8,最大 60)错开，但仍 <100",
                );
            }
        }
    }

    // ── `docs/ecl-ops.md` 号表 ↔ 常量的双向钉死（终审补，2026-07-31）────────────────

    /// `"000"` / `"140"` → `Some(0)` / `Some(140)`；其余（`---`、`300-330`、正文词）→ `None`。
    /// **只认三位补零形式**，这是号表列的排版约定（见 `ecl-ops.md` 0xx 族那条排版说明）。
    fn doc_num(tok: &str) -> Option<u16> {
        let t = tok.trim();
        (t.len() == 3 && t.bytes().all(|b| b.is_ascii_digit()))
            .then(|| t.parse().ok())
            .flatten()
    }

    /// 依次取出 cell 里所有反引号包起来的**原样** token（不判字符集，`$` 前缀剥掉）。
    fn backticked(cell: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = cell;
        while let Some(i) = rest.find('`') {
            let after = &rest[i + 1..];
            let Some(j) = after.find('`') else { break };
            out.push(after[..j].trim_start_matches('$'));
            rest = &after[j + 1..];
        }
        out
    }

    fn is_ident(s: &str) -> bool {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    /// 从 `docs/ecl-ops.md` 的 `## syscall 号表` 一节抽出全部 `(号, 名)` 对。
    ///
    /// 两个来源，都是文档**自己既有的**排版，不是为测试新加的标记：
    /// - **表行** `| 号 | 名 | 参数 | 返回 |`：首格是号（`NNN` 或 `NNN/NNN`），
    ///   次格开头是反引号名。`| 010/011 | `player_x/y` |` 这种合并写法按 `_` 词干展开。
    /// - **围栏块**里相邻的 `NNN name`（3xx 弹 setter 九连只在围栏里逐条列名，
    ///   表行是聚合的 `| 300-330 | 弹 setter 族 |`）。
    ///
    /// 第二个返回值 = 那些**没能配上名**的格子里的三位数（如 `300-330` 的两个端点），
    /// 只做"必须是真号"的弱检查。
    fn parse_doc_number_table() -> (Vec<(u16, String)>, Vec<u16>) {
        const DOC: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/ecl-ops.md"
        ));

        let start = DOC
            .find("\n## syscall 号表")
            .expect("ecl-ops.md 缺 `## syscall 号表` 节标题——文档结构变了，本测试要跟着改");
        let sec = &DOC[start + 1..];
        let end = sec[3..].find("\n## ").map_or(sec.len(), |i| i + 4);
        let sec = &sec[..end];

        let mut pairs: Vec<(u16, String)> = Vec::new();
        let mut loose: Vec<u16> = Vec::new();
        let mut in_fence = false;

        for line in sec.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                // 围栏：相邻两 token 形如 `NNN name`
                let toks: Vec<&str> = line.split_whitespace().collect();
                for w in toks.windows(2) {
                    if let (Some(n), true) = (doc_num(w[0]), is_ident(w[1])) {
                        pairs.push((n, w[1].to_string()));
                    }
                }
                continue;
            }
            let Some(body) = line.strip_prefix('|') else {
                continue;
            };
            let mut cells = body.split('|');
            let (Some(num_cell), Some(name_cell)) = (cells.next(), cells.next()) else {
                continue;
            };
            let nums: Vec<Option<u16>> = num_cell.trim().split('/').map(doc_num).collect();
            if nums.iter().any(Option::is_none) {
                // 聚合格（`300-330`）或表头分隔行（`---`）：只捞里面的三位数弱检查
                let bytes = num_cell.as_bytes();
                for i in 0..bytes.len() {
                    if bytes[i].is_ascii_digit()
                        && (i == 0 || !bytes[i - 1].is_ascii_digit())
                        && let Some(n) = doc_num(&num_cell[i..(i + 3).min(num_cell.len())])
                    {
                        loose.push(n);
                    }
                }
                continue;
            }
            let nums: Vec<u16> = nums.into_iter().flatten().collect();
            let ticked = backticked(name_cell);
            let mut names: Vec<String> = ticked
                .iter()
                .filter(|s| is_ident(s))
                .map(|s| s.to_string())
                .collect();
            // 合并写法 `player_x/y` → player_x / player_y
            if names.len() < nums.len()
                && let Some((a, b)) = ticked.first().and_then(|t| t.split_once('/'))
                && let Some((stem, _)) = a.rsplit_once('_')
            {
                names = vec![a.to_string(), format!("{stem}_{b}")];
            }
            assert!(
                names.len() >= nums.len(),
                "ecl-ops.md 号表行 `{}` 的号有 {} 个、认得出的反引号名只有 {} 个——\
                 排版变了就得改 `parse_doc_number_table`（别把这条测试关掉）",
                line.chars().take(60).collect::<String>(),
                nums.len(),
                names.len(),
            );
            for (n, name) in nums.into_iter().zip(names) {
                pairs.push((n, name));
            }
        }
        (pairs, loose)
    }

    /// **`docs/ecl-ops.md` 的号表必须与 `SYS_*` 常量两向一致**（终审补，2026-07-31）。
    ///
    /// 补的是一个**沉默漂移面**：`ecl-ops.md` 是号表层的权威呈现面（M4 对端与回放头要认的
    /// 冻结契约），而在此之前**仓内没有任何东西把它和常量绑住**——一次手滑编辑就能让文档
    /// 与代码分家，且不会有任何测试转红。号表重排那一刀是靠**手工写脚本**比对才确认干净的，
    /// 那种一次性验证保不住下一次。
    ///
    /// 两个方向都断言（缺一个就只是半张网）：
    /// - **文档 → 常量**：文档里出现的每个 `(号, 名)` 对都得在 [`frozen_table`] 里；
    /// - **常量 → 文档**：102 条常量每条都得在文档里出现，**漏记一条即红**。
    ///
    /// **它还有第二重职责，别只当它是"防文档漂移"**：本条是**全仓唯一**能抓到
    /// **族内互换**（号换了、族没换，如 `SYS_ATAN2` ↔ `SYS_DIST`）的测试。
    /// [`syscall_table_is_hundred_partitioned_and_unique`] 那张表**符号引用**常量，对具体
    /// 取值免疫；实测把这一对的值对调后，**全仓 947 条测试无一转红**（终审复跑，2026-07-31）。
    /// 本条按**字面数字**比对，是那一格唯一的网——变异实证见下（对调文档号列即红，
    /// 而结构测试照绿）。删本条 = 把那一格重新变瞎。
    ///
    /// 解析靠的是文档自己既有的排版（号列 + 反引号名），没有新加机器可读标记。排版真变了
    /// 本测试会**带着行内容响亮失败**并要求同步改解析器——号表是冻结面，这个代价是对的。
    #[test]
    fn ecl_ops_doc_syscall_numbers_match_the_constants() {
        let table = frozen_table();
        let (doc_pairs, loose) = parse_doc_number_table();

        assert!(
            doc_pairs.len() >= 79,
            "只从 ecl-ops.md 解析出 {} 个 (号,名) 对，远少于 79——解析器多半没跟上排版变更",
            doc_pairs.len()
        );

        // 方向一：文档有而常量无（含"号对名错"与"名对号错"）
        for (num, name) in &doc_pairs {
            assert!(
                table.iter().any(|&(n, nm, _)| n == *num && nm == name),
                "ecl-ops.md 记着 `{num} = {name}`，但 `SYS_*` 常量里没有这一对；\
                 号表以 `ecl::syscall` 的常量为唯一权威，改文档别改代码"
            );
        }

        // 方向二：常量有而文档无（漏记一条）
        for &(num, name, _) in table {
            assert!(
                doc_pairs.iter().any(|(n, nm)| *n == num && nm == name),
                "`SYS_{}` = {num} 在 `docs/ecl-ops.md` 的号表里查无此条——新增/改号必须同步文档",
                name.to_uppercase()
            );
        }

        // 聚合格（如 `| 300-330 | 弹 setter 族 |`）的端点也得是真号
        for n in loose {
            assert!(
                syscall_implemented(n),
                "ecl-ops.md 号表的聚合格里出现了 {n}，但它不是任何 syscall 的号"
            );
        }
    }

    /// 派发测试助手：把 `args`（脚本**声明顺序**，正序）压栈，直连 `dispatch`（不经
    /// `OP_SYS`/`exec`——聚焦 syscall 语义本身，`OP_SYS` 派发链路已由 `vm.rs` 测试覆盖）。
    fn call(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
    ) -> Result<(), u8> {
        call_with_tables(w, ecl, task, no, args, &TABLES_V0)
    }

    /// 绑定**指定表**的 `call`——空格判据一类"内建表提供不了靶子"的测试用它挂合成表
    /// （当前图集 12 行全满 16 色，没有空格；数据如实反映美术，不为测试在内建表里造假）。
    fn call_with_tables(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
        tables: &crate::tables::WorldTables,
    ) -> Result<(), u8> {
        call_full(w, ecl, task, no, args, tables, 0)
    }

    /// 指定 `self_index` 的 `call`——shooter 族专用（复审 ③）：shooter 存储按**任务索引**
    /// 键，而 `call` 把 `self_index` 写死 0，于是"派发臂用的是 `ctx.self_index` 还是字面量
    /// 0"在测试里不可辨（等价变异）。`vm.rs` 已有 `self_index: 7` 的先例。
    fn call_at(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
        self_index: u16,
    ) -> Result<(), u8> {
        call_full(w, ecl, task, no, args, &TABLES_V0, self_index)
    }

    fn call_full(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
        tables: &crate::tables::WorldTables,
        self_index: u16,
    ) -> Result<(), u8> {
        for &a in args {
            task.stack[task.sp as usize] = a;
            task.sp += 1;
        }
        let mut budget = u32::MAX;
        let frame = w.body.frame;
        let mut ctx = VmCtx {
            code: &[],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl,
            body: &mut w.body,
            tables,
            self_index,
            frame,
        };
        dispatch(no, task, &mut ctx)
    }

    /// 合成一张"某格是空格"的表（其余与内建表逐位相同）。
    fn tables_with_hole(hole: usize) -> crate::tables::WorldTables {
        let mut t = crate::tables::build_tables_v0();
        let mut rows = t.appearances.to_vec();
        rows[hole].valid = false;
        t.appearances = rows.into_boxed_slice();
        t
    }

    fn fresh() -> (Box<World>, EclImage) {
        (World::new(1), EclImage::empty())
    }

    /// 打包敌号 → 池槽索引（敌句柄打包刀 2026-07-31）。`spawn_enemy` 押的不再是裸 index，
    /// 想拿"哪个槽"去戳池内存的测试走这里；想拿"敌号"喂读口的测试用
    /// [`crate::enemy::pack_handle`]。
    fn enemy_slot(packed: i32) -> usize {
        (packed & 0xFFFF) as usize
    }

    /// 单个 0 参 Async sub（raw=1）的镜像——`spell_begin` 的 `pattern:SubRef` 测试专用
    /// （代码体本身无关紧要，只需能被 `run_tasks` 安全 wait）。
    fn async_pattern_image() -> EclImage {
        test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                1,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("pattern", 1)],
            Some(0),
        )
    }

    /// 单个 **CallOnly**（非 Async）sub（raw=1）的镜像——task 号支路③专用（B25）：
    /// 号在册（`sub_id`/`sub_meta` 均命中）但 `kind() != SubKind::Async`，用来钉死
    /// "在册但不是零参 Async" 这条校验，而不是被支路②（号不在册）顺手拦下。
    /// CallOnly sub 不需要 entry 表项（`MissingAsyncEntry` 只查 Async sub）。
    fn wrong_kind_task_image() -> EclImage {
        test_image(
            vec![crate::ecl::ops::OP_RET as u32],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::CallOnly, vec![]),
            ],
            vec![],
            Some(0),
        )
    }

    /// 单个**带参** `Async` sub（raw=1，一个 `Fx` 参）的镜像——支路③另一半专用（终审 M-4）：
    /// `kind() != Async || !param_types().is_empty()` 这个判据是**或**，`wrong_kind_task_image`
    /// 只打了左半（kind 不对）；这个 helper 打右半（kind 对但带参）——号在册、`sub_meta`
    /// 命中、`kind()==Async`，唯独 `param_types` 非空。若把 `!param_types().is_empty()`
    /// 那半判据删掉，这里必须能把变异逮住（否则"带参 Async sub 不许当 task 挂载"这条
    /// 约束就是无网状态）。先例：`image.rs:606` 的 `SubInit::new(0, SubKind::Async,
    /// vec![EclValueType::Fx])`。
    fn async_with_param_task_image() -> EclImage {
        test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                1,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![crate::ecl::image::EclValueType::Fx]),
            ],
            vec![EntryInit::new("pattern", 1)],
            Some(0),
        )
    }

    #[test]
    fn sys_frame_reads_world_frame() {
        let (mut w, ecl) = fresh();
        w.body.frame = 123;
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_FRAME, &[]).is_ok());
        assert_eq!(task.stack[0], 123);
    }

    #[test]
    fn sys_player_xy_reads_player0_position() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_PLAYER_X, &[]).is_ok());
        assert_eq!(task.stack[0], w.body.players[0].x.raw());
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_PLAYER_Y, &[]).is_ok());
        assert_eq!(task.stack[0], w.body.players[0].y.raw());
    }

    /// self_x/self_y/self_hp 三态：ENEMY→敌池读、BULLET→弹池位置(hp 恒 0)、STAGE→恒 0。
    #[test]
    fn sys_self_reads_dispatch_by_owner_kind() {
        let (mut w, ecl) = fresh();
        let eh = w.body.create_enemy(crate::enemy::EnemyInit {
            x: Fx::from_int(11),
            y: Fx::from_int(22),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
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
            hp: 7,
            hp_max: 7,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            anm_state_frame: 0,
            main_task: 0,
            death_script: 0,
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
            score: 0,
        });
        let mut enemy_task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_X, &[]).is_ok());
        assert_eq!(enemy_task.stack[0], Fx::from_int(11).raw());
        enemy_task.sp = 0;
        assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_Y, &[]).is_ok());
        assert_eq!(enemy_task.stack[0], Fx::from_int(22).raw());
        enemy_task.sp = 0;
        assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_HP, &[]).is_ok());
        assert_eq!(enemy_task.stack[0], 7);

        let bh = crate::world::test_support::bullet_at(&mut w, 33, 44);
        let mut bullet_task = Task {
            owner_kind: OWNER_BULLET,
            owner_index: bh.index,
            owner_gen: bh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut bullet_task, SYS_SELF_X, &[]).is_ok());
        assert_eq!(bullet_task.stack[0], Fx::from_int(33).raw());
        bullet_task.sp = 0;
        assert!(call(&mut w, &ecl, &mut bullet_task, SYS_SELF_HP, &[]).is_ok());
        assert_eq!(bullet_task.stack[0], 0, "弹 hp 读恒 0");

        let mut stage_task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut stage_task, SYS_SELF_X, &[]).is_ok());
        assert_eq!(stage_task.stack[0], 0, "STAGE self_x 恒 0");
    }

    /// 四个 $self_* 在敌 owner 下读到池字段。取 (3, -7) 这个 x≠y 且异号的速度——
    /// 派发臂写反立刻可辨。
    #[test]
    fn self_velocity_vars_read_enemy_pool() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let i = eh.index as usize;
        w.body.enemies.vx[i] = Fx::from_int(3);
        w.body.enemies.vy[i] = Fx::from_int(-7);
        w.body.enemies.speed[i] = Fx::from_int(9);
        w.body.enemies.angle[i] = crate::math::Angle::QUARTER;
        let mk = || Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        for (sys, want) in [
            (SYS_SELF_VX, Fx::from_int(3).raw()),
            (SYS_SELF_VY, Fx::from_int(-7).raw()),
            (SYS_SELF_SPEED, Fx::from_int(9).raw()),
            (SYS_SELF_ANGLE, crate::math::Angle::QUARTER.raw() as i32),
        ] {
            let mut task = mk();
            assert!(call(&mut w, &ecl, &mut task, sys, &[]).is_ok());
            assert_eq!(pop(&mut task).unwrap(), want, "syscall {sys}");
        }
    }

    /// 弹 owner 下同样有效（弹池本就有这四个字段——双表示是从弹抄来的）。
    #[test]
    fn self_velocity_vars_dispatch_to_bullet_pool() {
        let (mut w, ecl) = fresh();
        let bh = crate::world::test_support::bullet_at(&mut w, 0, 0);
        let i = bh.index as usize;
        w.body.bullets.vx[i] = Fx::from_int(2);
        let mut task = Task {
            owner_kind: OWNER_BULLET,
            owner_index: bh.index,
            owner_gen: bh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_SELF_VX, &[]).is_ok());
        assert_eq!(pop(&mut task).unwrap(), Fx::from_int(2).raw());
    }

    /// CART_FX 惰性化（引擎第二刀 §5）：弹积分 1 帧后 `speed`/`angle` 是陈值（标脏），
    /// 读 `$self_angle` 必须先 materialize —— 值等于 `atan2(vy,vx)`，且**读取会顺带回写**
    /// （读完 `BULLET_POLAR_STALE` 应已清零）。
    #[test]
    fn self_angle_read_materializes_stale_cart_fx_bullet() {
        let (mut w, ecl) = fresh();
        let bh = crate::world::test_support::bullet_at(&mut w, 0, 200);
        let i = bh.index as usize;
        w.body.bullets.vx[i] = Fx::from_int(3);
        w.body.bullets.vy[i] = Fx::from_int(4);
        w.body.set_bullet_gravity(bh, Fx::ZERO, Fx::from_raw(16384)); // 开 CART_FX
        let frame = w.body.frame;
        crate::step::step(
            &mut w,
            &TABLES_V0,
            &ecl,
            &crate::input::InputFrame::empty(frame),
        );
        assert_ne!(
            w.body.bullets.flags[i] & crate::bullets::BULLET_POLAR_STALE,
            0,
            "CART_FX 积分一帧后应标脏"
        );
        let (vx, vy) = (w.body.bullets.vx[i], w.body.bullets.vy[i]);
        let mut task = Task {
            owner_kind: OWNER_BULLET,
            owner_index: bh.index,
            owner_gen: bh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ANGLE, &[]).is_ok());
        assert_eq!(
            pop(&mut task).unwrap(),
            crate::math::cordic::atan2(vy, vx).raw() as i32,
            "$self_angle 应等于 atan2(vy,vx)"
        );
        assert_eq!(
            w.body.bullets.flags[i] & crate::bullets::BULLET_POLAR_STALE,
            0,
            "读取应顺带回写，清掉陈值位"
        );
    }

    /// 非敌非弹 owner → 0（同 $self_x 的既有降级）。
    #[test]
    fn self_velocity_vars_degrade_to_zero_for_stage_owner() {
        let (mut w, ecl) = fresh();
        for sys in [SYS_SELF_VX, SYS_SELF_VY, SYS_SELF_SPEED, SYS_SELF_ANGLE] {
            let mut task = Task {
                owner_kind: OWNER_STAGE,
                ..Task::default()
            };
            assert!(call(&mut w, &ecl, &mut task, sys, &[]).is_ok());
            assert_eq!(pop(&mut task).unwrap(), 0, "syscall {sys} 非敌非弹应降级 0");
        }
    }

    #[test]
    fn sys_get_set_var_roundtrip() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // SET_VAR(slot=20, val=42) —— 正序压栈 slot,val；slot=20 落自由段（≥
        // GLOBALS_SYS_SEGMENT=16），非本测试关注点的系统段写保护见
        // `sys_set_var_system_segment_slot_is_guarded_no_op` 单开测试。
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[20, 42]).is_ok());
        assert_eq!(w.body.globals[20], 42);
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_GET_VAR, &[20]).is_ok());
        assert_eq!(task.stack[0], 42);
    }

    /// globals 段纪律（甲案，M1.5）：`SYS_SET_VAR` 写系统段（slot < `GLOBALS_SYS_SEGMENT`=16）
    /// → **no-op**（真槽值不变）+ `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不 Fault**
    /// （`dispatch` 仍返回 `Ok(())`——P4-b 脚本作者违约的确定性安全结果，非引擎 bug，调度层不杀
    /// 任务）。边界精确钉死：slot=15（系统段最高位）挡、slot=16（自由段最低位）放行。
    /// `SYS_GET_VAR` 两段皆不受限（读无保护）。世界 API `set_var` 直写不经此guard（仍可写系统段
    /// ——game 层建场用它写 RANK，见 `world::GLOBALS_SYS_SEGMENT` 文档）。
    #[test]
    fn sys_set_var_system_segment_slot_is_guarded_no_op() {
        let (mut w, ecl) = fresh();
        w.body.set_var(crate::world::GVAR_RANK, 111); // 世界 API 先写系统段一个已知值（建场惯例）
        let mut task = Task::default();

        // 脚本经 SYS_SET_VAR 写 slot=0（系统段）→ no-op，dispatch 仍 Ok（非 Fault）。
        assert!(
            call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[0, 999]).is_ok(),
            "系统段写保护是 no-op，不是 Fault"
        );
        assert_eq!(w.body.globals[0], 111, "真槽值不变（未被脚本 999 覆盖）");
        assert_eq!(w.body.diag.contract_viol, 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);

        // 边界：slot=15 仍属系统段 → 同样挡。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[15, 999]).is_ok());
        assert_eq!(w.body.globals[15], 0, "slot=15 系统段边界仍挡");
        assert_eq!(w.body.diag.contract_viol, 2);

        // 边界：slot=16 是自由段最低位 → 正常写入（guard 不越界误伤）。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[16, 999]).is_ok());
        assert_eq!(w.body.globals[16], 999, "slot=16 自由段最低位正常写");
        assert_eq!(w.body.diag.contract_viol, 2, "自由段写不应额外计数");

        // 世界 API `set_var` 直写系统段不受本 guard 影响（不同门，见模块文档）。
        w.body.set_var(crate::world::GVAR_RANK, 777);
        assert_eq!(w.body.globals[0], 777, "世界 API 写系统段仍畅通");

        // `SYS_GET_VAR` 读系统段不受限。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_GET_VAR, &[0]).is_ok());
        assert_eq!(task.stack[0], 777, "脚本读系统段不受限");
    }

    /// 任务龄 `SYS_SELF_AGE`（M1.5）：`ctx.frame - task.born_frame`（wrapping，i32 域）——
    /// 纯 dispatch 层直调（不经调度门禁），钉死算式本身；跨帧调度语义的端到端 off-by 判别见
    /// `step.rs` 的 `sys_self_age_ticks_task_age_each_frame_via_globals_relay`。
    #[test]
    fn sys_self_age_computes_frame_minus_born_frame() {
        let (mut w, ecl) = fresh();
        let mut task = Task {
            born_frame: 10,
            ..Task::default()
        };
        w.body.frame = 13;
        assert!(call(&mut w, &ecl, &mut task, SYS_SELF_AGE, &[]).is_ok());
        assert_eq!(task.stack[0], 3, "13-10=3");

        // wrapping：born_frame 数值上"晚于" frame（理论不该发生，但算式必须 wrapping 不 panic）。
        let mut task2 = Task {
            born_frame: 5,
            ..Task::default()
        };
        w.body.frame = 0;
        assert!(call(&mut w, &ecl, &mut task2, SYS_SELF_AGE, &[]).is_ok());
        assert_eq!(task2.stack[0], 0i32.wrapping_sub(5), "wrapping 不 panic");
    }

    /// owner 上限血量 `SYS_SELF_HP_MAX`（M1.5）：ENEMY→`hp_max` 字段（与 `hp` 不同值，逐位命中
    /// 排除"读混 hp/hp_max 两个同族字段"的变异）；非敌（STAGE/BULLET）→ 恒 0（同 `SYS_SELF_HP`
    /// 误用策略）。
    #[test]
    fn sys_self_hp_max_dispatch_by_owner_kind() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let i = eh.index as usize;
        w.body.enemies.hp_max[i] = 9999; // 与 hp=5 明显不同，逐位命中排除读混字段
        let mut enemy_task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_HP_MAX, &[]).is_ok());
        assert_eq!(enemy_task.stack[0], 9999);

        let mut stage_task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut stage_task, SYS_SELF_HP_MAX, &[]).is_ok());
        assert_eq!(stage_task.stack[0], 0, "非敌 owner 恒 0");
    }

    /// `n==0` → 押 0、不消耗 RNG 流（拍板钉死：连续两次 n=0 调用后世界 RNG 状态与初始一致，
    /// 通过"随后一次真实 rand_range 调用的结果与从未调用过 n=0 的另一世界一致"来判别）。
    #[test]
    fn sys_rand_range_zero_does_not_consume_rng() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_RAND_RANGE, &[0]).is_ok());
        assert_eq!(task.stack[0], 0, "n=0 押 0");
        let after_zero_calls = w.body.rng;
        let mut w2 = World::new(1);
        assert_eq!(
            after_zero_calls, w2.body.rng,
            "n=0 不消耗 RNG 流——与全新同种子世界的 RNG 状态相同"
        );
        // 消耗判别式的另一半：真实调用（n>0）必须真的推进流，且与直接调 rand_range 序列一致。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_RAND_RANGE, &[100]).is_ok());
        let direct = w2.body.rng.rand_range(100) as i32;
        assert_eq!(
            task.stack[0], direct,
            "syscall 消费的 RNG 流与直接调用序列一致"
        );
    }

    /// B15 覆盖缺口补齐：负 `n` 与 `n==0` 走同一分支（`if n <= 0`），但从未有测试独立
    /// 拿负数实测过——这里钉死同样的判别式（不消耗 RNG 流 + 押 0），防止未来有人把
    /// 判别条件悄悄改成 `n == 0`（丢负数分支）而没有任何测试炸。
    #[test]
    fn sys_rand_range_negative_n_does_not_consume_rng() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_RAND_RANGE, &[-5]).is_ok());
        assert_eq!(task.stack[0], 0, "负 n 押 0，同 n=0");
        let after_negative_call = w.body.rng;
        let w2 = World::new(1);
        assert_eq!(
            after_negative_call, w2.body.rng,
            "负 n 不消耗 RNG 流——与全新同种子世界的 RNG 状态相同"
        );
    }

    /// 丙方案 create_bullet：dumb 路径（xform_cnt=0）——位置精确、appearance 半径/sprite
    /// 逐位查表命中。
    #[test]
    fn sys_create_bullet_dumb_path_position_and_appearance_bitwise() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 正序：appearance,x,y,speed,angle,xform_off,xform_cnt,task_script
        let args = [
            ROW_B,
            Fx::from_int(10).raw(),
            Fx::from_int(-20).raw(),
            Fx::from_int(3).raw(),
            Angle::QUARTER.raw() as i32,
            0,
            0,
            -1,
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
        let idx = task.stack[0];
        assert!(idx >= 0, "创建成功压回句柄 index");
        let i = idx as usize;
        assert_eq!(w.body.bullets.x[i], Fx::from_int(10));
        assert_eq!(w.body.bullets.y[i], Fx::from_int(-20));
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3));
        assert_eq!(w.body.bullets.angle[i], Angle::QUARTER);
        let cfg = &TABLES_V0.appearances[ROW_B as usize];
        assert_eq!(
            w.body.bullets.radius[i], cfg.radius,
            "appearance 半径逐位命中"
        );
        assert_eq!(
            w.body.bullets.sprite[i], cfg.sprite,
            "appearance sprite 逐位命中"
        );
        assert_eq!(
            w.body.bullets.transform_head[i],
            crate::xform::XFORM_NONE,
            "xform_cnt=0 → 哑弹"
        );
    }

    #[test]
    fn sys_create_bullet_appearance_oob_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let bad_appearance = TABLES_V0.appearances.len() as i32; // 恰过界
        let args = [bad_appearance, 0, 0, 0, 0, 0, 0, -1];
        let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "校验先于创建，零副作用"
        );
    }

    /// 空格 appearance（图集该格没有图）→ Fault，且**弹未被创建**（先验后建）。
    /// 这是"隐形弹"（有判定无图像）的运行期闸；编译期同款判据见 lang::typeck。
    #[test]
    fn sys_create_bullet_blank_cell_faults_without_creating() {
        const BLANK: i32 = 3 * 16 + 7; // 合成表里挖的那一格
        let holed = tables_with_hole(BLANK as usize);
        assert!(
            !holed.appearances[BLANK as usize].valid && TABLES_V0.appearances[BLANK as usize].valid,
            "前提：{BLANK} 在合成表里是空格、在内建表里不是——否则本测试无判别力"
        );
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = [BLANK, 0, 0, 0, 0, 0, 0, -1];
        let r = call_with_tables(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args, &holed);
        assert_eq!(r, Err(FAULT_BAD_OP), "空格 appearance 必须 Fault");
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "Fault 时不得留下半成品弹（先验后建）"
        );
    }

    /// xform 区间引用解包逐位：locals 手填 3 词槽（word0=(wait<<16)|(op<<8)，word1/2=args），
    /// 创建后段内容与手搭 `XformSlot` 逐位相等。
    #[test]
    fn sys_create_bullet_xform_unpack_bitwise() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 槽 0：wait=3, op=OP_SET_SPEED(10), args=[Fx::from_int(2).raw(), 0]
        let off = 4usize;
        task.locals[off] = ((3u32 << 16) | (crate::xform::OP_SET_SPEED as u32) << 8) as i32;
        task.locals[off + 1] = Fx::from_int(2).raw();
        task.locals[off + 2] = 0;
        let args = [
            ROW_A, 0, 0, 0, 0, off as i32, 1, // xform_cnt=1
            -1,
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
        let idx = task.stack[0] as usize;
        let seg = w.body.bullets.transform_head[idx];
        assert_ne!(seg, crate::xform::XFORM_NONE, "xform_cnt>0 → 带段");
        let expect = XformSlot {
            wait: 3,
            op: crate::xform::OP_SET_SPEED,
            _pad: 0,
            args: [Fx::from_int(2).raw(), 0],
        };
        assert_eq!(w.body.xforms.seg_slots(seg)[0], expect, "解包逐位相等");
    }

    /// off+cnt*3 越出 LOCALS(64) → Fault，零副作用（不建弹）。
    #[test]
    fn sys_create_bullet_xform_bounds_violation_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = [
            ROW_A, 0, 0, 0, 0, 60, // off
            2,  // cnt：60+2*3=66 > LOCALS(64)
            -1,
        ];
        let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
    }

    /// 坏 task_script 号（不在册）→ Fault，且先查后建：弹不应被创建（零副作用）。
    #[test]
    fn sys_create_bullet_bad_task_script_faults_before_creating() {
        let (mut w, ecl) = fresh(); // subs 空——任何脚本号都越界
        let mut task = Task::default();
        let args = [ROW_A, 0, 0, 0, 0, 0, 0, 0]; // task_script=0 越界
        let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
    }

    /// P4-b 支路①（B25）：`fire`（`sys_create_bullet`）的 task_script 超出 `u16` 范围
    /// （`u16::try_from` 失败）→ Fault，且先查后建：弹不应被创建（零副作用）。
    /// 镜像 `spawn_enemy_task_script_out_of_u16_faults_without_enemy`。
    ///
    /// **取值刻意挑过**（变异检验陷阱，见 task-3 报告）：`65537 as u16 == 1`，而
    /// `1` 在 `async_pattern_image()` 里恰是一个合法在册的零参 Async sub——若把
    /// `u16::try_from` 检查误删成 `as u16` 截断，这条参数会"活下来"变成合法号，
    /// 从而真的建成弹（支路②③都拦不住它）。用一个恰好越界又恰好在册的取值，才能
    /// 让"删掉①检查"这个变异被**这条测试**真正捕捉，而不是被②顺手挡住。
    #[test]
    fn sys_create_bullet_task_script_out_of_u16_faults_before_creating() {
        let ecl = async_pattern_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        // 正序：appearance,x,y,speed,angle,xform_off,xform_cnt,task_script
        let args = [ROW_A, 0, 0, 0, 0, 0, 0, 65537]; // 65537 > u16::MAX，但截断后=1（在册）
        let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
    }

    /// P4-b 支路③（B25）：`fire` 的 task_script 号在册但不是零参 `Async`（这里用
    /// `CallOnly`）→ Fault，且先查后建：弹不应被创建（零副作用）。镜像
    /// `spawn_enemy_task_script_wrong_kind_faults_without_enemy`。
    #[test]
    fn sys_create_bullet_task_script_wrong_kind_faults_before_creating() {
        let ecl = wrong_kind_task_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        let args = [ROW_A, 0, 0, 0, 0, 0, 0, 1]; // task_script=1，在册但 CallOnly
        let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
    }

    /// P4-b 支路③另一半（终审 M-4）：`fire` 的 task_script 号在册、`kind()==Async`，
    /// 唯独**带参**（非零参）→ 同样必须 Fault，且先查后建（弹不应被创建）。
    /// 与 `sys_create_bullet_task_script_wrong_kind_faults_before_creating` 互补——
    /// 那条打 kind 半边，这条打 param_types 半边，合起来才覆盖判据的完整析取。
    /// 镜像 `spawn_enemy_task_script_with_params_faults_without_enemy`。
    #[test]
    fn sys_create_bullet_task_script_with_params_faults_before_creating() {
        let ecl = async_with_param_task_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        let args = [ROW_A, 0, 0, 0, 0, 0, 0, 1]; // task_script=1，在册、Async，但带参
        let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
    }

    /// 池满 → 押 -1，不 Fault，不派任务（NULL 分支）。
    #[test]
    fn sys_create_bullet_pool_full_pushes_neg1() {
        let (mut w, ecl) = fresh();
        for _ in 0..crate::bullets::BulletPool::CAP {
            w.body.create_bullet(crate::bullets::BulletInit {
                x: Fx::ZERO,
                y: Fx::ZERO,
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                speed: Fx::ZERO,
                angle: Angle::ZERO,
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
                born_frame: 0,
            });
        }
        let mut task = Task::default();
        let args = [ROW_A, 0, 0, 0, 0, 0, 0, -1];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
        assert_eq!(task.stack[0], -1, "池满押 -1");
    }

    /// task_script ≥ 0 挂弹派任务：owner=(BULLET, 新弹 index/gen)，`parent`/`born_frame` 戳记
    /// 与 `OP_SPAWN` 同款；弹死后任务次帧被静默回收（owner 门禁，非 Fault）。
    #[test]
    fn sys_create_bullet_task_script_spawns_owner_bound_task_and_dies_with_bullet() {
        let ecl = test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("bullet_task", 1)],
            Some(0),
        );
        let mut w = World::new(1);
        w.body.frame = 5;
        let mut task = Task::default();
        let args = [ROW_A, 0, 0, 0, 0, 0, 0, 1]; // task_script=1（在册）
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
        let bidx = task.stack[0] as u16;
        let bgen = w.body.bullets.generation[bidx as usize];

        // 恰新增一个任务，owner 绑定新弹，born_frame 戳为 ctx.frame（=5，次帧首跑）。
        let child = (0..crate::ecl::task::TASK_CAP)
            .find(|&i| w.tasks.is_alive(i))
            .expect("应已派生子任务");
        assert_eq!(w.tasks.slots[child].owner_kind, OWNER_BULLET);
        assert_eq!(w.tasks.slots[child].owner_index, bidx);
        assert_eq!(w.tasks.slots[child].owner_gen, bgen);
        assert_eq!(w.tasks.slots[child].born_frame, 5);

        // 弹死 → 次帧任务被静默回收（owner 门禁，通过完整 step 驱动验证端到端链路）。
        w.body.bullets.free(crate::bullets::BulletHandle {
            index: bidx,
            generation: bgen,
        });
        let frame = w.body.frame;
        crate::step::step(
            &mut w,
            &TABLES_V0,
            &ecl,
            &crate::input::InputFrame::empty(frame),
        );
        assert!(!w.tasks.is_alive(child), "owner 弹死后任务应被静默回收");
        assert_eq!(w.body.diag.task_faults, 0, "owner 死不是 Fault");
    }

    #[test]
    fn sys_create_bullets_batch_ring_count_and_appearance() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 正序：appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step
        let args = [
            ROW_C,
            0,
            Fx::from_int(100).raw(),
            8,
            0,
            8192,
            1,
            Fx::from_int(2).raw(),
            0,
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
        assert_eq!(task.stack[0], 8, "8-way 环实发 8");
        assert_eq!(w.body.bullets.iter_alive().count(), 8);
        let cfg = &TABLES_V0.appearances[ROW_C as usize];
        for i in 0..8 {
            assert_eq!(w.body.bullets.radius[i], cfg.radius);
            assert_eq!(w.body.bullets.sprite[i], cfg.sprite);
        }
    }

    /// 批量入口同款（两族同构，别只补一边）：空格 appearance → Fault，且**弹未被创建**
    /// （先验后建）；编译期同款判据见 lang::typeck。
    #[test]
    fn sys_create_bullets_batch_blank_cell_faults_without_creating() {
        const BLANK: i32 = 3 * 16 + 7; // 同上,合成表
        let holed = tables_with_hole(BLANK as usize);
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 正序：appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step
        let args = [BLANK, 0, 0, 4, 0, 4096, 1, Fx::from_int(1).raw(), 0];
        let r = call_with_tables(
            &mut w,
            &ecl,
            &mut task,
            SYS_CREATE_BULLETS_BATCH,
            &args,
            &holed,
        );
        assert_eq!(r, Err(FAULT_BAD_OP), "空格 appearance 必须 Fault");
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "Fault 时不得留下半成品弹（先验后建）"
        );
    }

    #[test]
    fn sys_spawn_enemy_creates_with_fields() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 正序：x,y,hp,drop_table,score,sprite,task(none=-1)（A5 乙案：7 参，尾追 sprite/task）
        let args = [
            Fx::from_int(5).raw(),
            Fx::from_int(6).raw(),
            42,
            1,
            100,
            0,
            -1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let handle = task.stack[0];
        assert!(handle >= 0);
        let i = enemy_slot(handle);
        assert_eq!(w.body.enemies.x[i], Fx::from_int(5));
        assert_eq!(w.body.enemies.y[i], Fx::from_int(6));
        assert_eq!(w.body.enemies.hp[i], 42);
        // 表号在生成时就展开成逐类型计数（表 1 = POWER×2 + POINT×1）。
        assert_eq!(
            w.body.enemies.drop_count[i],
            crate::tables::drop_counts(&TABLES_V0, 1).0
        );
        assert_eq!(
            w.body.enemies.drop_count[i][crate::items::ITEM_POWER as usize],
            2
        );
        assert_eq!(w.body.enemies.score[i], 100);
    }

    /// P4-b：`spawn_enemy` 的越界 `drop_table` → 视同空表 + 计 contract_viol +
    /// `last_status=BAD_ARGS`，敌照建、不 panic、死时不掉道具
    /// （原 B11，随掉落状态从 settle 前移到生成时）。
    /// 负数表号同样走这条（`as u16` 回绕成大正数 → 仍越界）。
    ///
    /// `last_status` 那条断言是全支线复审 Minor #6 补的：兄弟测试
    /// `world::tests::enemy_drop_and_kill_apis_degrade_on_stale_handle` 一直断言两样，
    /// 这条搬到 syscall 层时只剩了 `contract_viol`。
    #[test]
    fn spawn_enemy_out_of_range_drop_table_degrades_to_empty() {
        // 内建表只有 2 张掉落表；取一个必然越界的正数号，再取 -1 走回绕那条腿。
        let oob = TABLES_V0.drop_tables.len() as i32 + 9;
        for &bad in &[oob, -1] {
            let (mut w, ecl) = fresh();
            let mut task = Task::default();
            let viol_before = w.body.diag.contract_viol;
            assert_eq!(
                w.body.last_status,
                crate::world::STATUS_OK,
                "前提：开局 last_status 干净（表号 {bad}）"
            );
            // 正序：x,y,hp,drop_table,score,sprite,task(none=-1)
            let args = [
                Fx::ZERO.raw(),
                Fx::from_int(80).raw(),
                10,
                bad,
                100,
                0,
                -1,
                0,
            ];
            assert!(
                call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok(),
                "越界表号不得 Fault（P4-b 降级，不是违约方的锅）"
            );
            let handle = task.stack[0];
            assert!(handle >= 0, "敌照建（表号 {bad}）");
            assert_eq!(
                w.body.diag.contract_viol,
                viol_before + 1,
                "越界表号须计一次违约（表号 {bad}）"
            );
            assert_eq!(
                w.body.last_status,
                crate::world::STATUS_BAD_ARGS,
                "越界表号须写 last_status（同邻居 P4-b 口径；表号 {bad}）"
            );
            assert_eq!(
                w.body.enemies.drop_count[enemy_slot(handle)],
                [0u8; crate::items::ITEM_TYPE_COUNT],
                "视同空表（表号 {bad}）"
            );
        }
    }

    /// task_script ≥0 挂敌派任务：owner=(ENEMY, 新敌 index/gen)，`main_task` 回填槽号+1。
    /// sprite 传判别值 5（非默认 0）——S1 纪律：圆心重合式测试对字段映射是瞎的，钉非默认值。
    #[test]
    fn spawn_enemy_with_task_binds_owner_and_main_task() {
        let ecl = test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("enemy_task", 1)],
            Some(0),
        );
        let mut w = World::new(1);
        w.body.frame = 3;
        let mut task = Task::default();
        // 正序：x,y,hp,drop_table,score,sprite,task_script；sprite=5（判别值），task=1（在册）
        let args = [
            Fx::from_int(0).raw(),
            Fx::from_int(80).raw(),
            10,
            0,
            0,
            5,
            1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let eidx = enemy_slot(task.stack[0]) as u16;
        assert_eq!(
            w.body.enemies.sprite[eidx as usize], 5,
            "sprite 判别值应落池（S1：非默认判别）"
        );
        let egen = w.body.enemies.generation[eidx as usize];

        let child = (0..crate::ecl::task::TASK_CAP)
            .find(|&i| w.tasks.is_alive(i))
            .expect("应已派生子任务");
        assert_eq!(w.tasks.slots[child].owner_kind, OWNER_ENEMY);
        assert_eq!(w.tasks.slots[child].owner_index, eidx);
        assert_eq!(w.tasks.slots[child].owner_gen, egen);
        assert_eq!(w.tasks.slots[child].born_frame, 3);
        assert_eq!(
            w.body.enemies.main_task[eidx as usize],
            child as u32 + 1,
            "main_task 应回填为任务槽号+1"
        );
    }

    /// task_script = -1（none）：敌建成、不派任何任务，`main_task` 保持 0。
    #[test]
    fn spawn_enemy_task_none_leaves_main_task_zero() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = [
            Fx::from_int(0).raw(),
            Fx::from_int(80).raw(),
            10,
            0,
            0,
            0,
            -1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let handle = task.stack[0];
        assert!(handle >= 0);
        assert_eq!(
            w.body.enemies.main_task[enemy_slot(handle)],
            0,
            "task=none 不应回填 main_task"
        );
        assert_eq!(
            (0..crate::ecl::task::TASK_CAP)
                .filter(|&i| w.tasks.is_alive(i))
                .count(),
            0,
            "task=none 不应派生任何任务"
        );
    }

    /// 坏 task_script 号（不在册）→ Fault，且先验后建：敌不应被创建（零副作用），
    /// 镜像 `sys_create_bullet_bad_task_script_faults_before_creating`。
    #[test]
    fn spawn_enemy_bad_task_script_faults_without_enemy() {
        let (mut w, ecl) = fresh(); // subs 空——任何脚本号都越界
        let mut task = Task::default();
        let args = [
            Fx::from_int(0).raw(),
            Fx::from_int(80).raw(),
            10,
            0,
            0,
            0,
            9999, // 不在册
            0,    // argc（boss 换段刀：210 调用约定追加）
        ];
        let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.enemies.iter_alive().count(), 0, "先验后建：零副作用");
    }

    /// P4-b 支路①：task 号超出 u16 范围 → Fault，且敌未创建（B25）。
    ///
    /// **取值刻意挑过**（同 `sys_create_bullet_task_script_out_of_u16_faults_before_creating`
    /// 的陷阱说明）：`65537 as u16 == 1`，而 `1` 在 `async_pattern_image()` 里恰是一个
    /// 合法在册的零参 Async sub——若把 `u16::try_from` 检查误删成 `as u16` 截断，这条
    /// 参数会"活下来"变成合法号，敌真的会建成（支路②③都拦不住）。选一个恰好越界又
    /// 恰好截断落进"在册"的取值，才能让这条测试真正命中支路①，而不是被②顺手挡住。
    #[test]
    fn spawn_enemy_task_script_out_of_u16_faults_without_enemy() {
        let ecl = async_pattern_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        // 正序参：x, y, hp, drop_table, score, sprite, task_script
        let args = [0, 0, 100, 0, 0, 0, 65537, 0]; // 65537 > u16::MAX，但截断后=1（在册）
        let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(
            w.body.enemies.iter_alive().count(),
            0,
            "先验后建：不留半成品"
        );
    }

    /// P4-b 支路③：task 号在册但不是零参 `Async`（这里用 `CallOnly`）→ Fault，
    /// 且敌未创建（B25）。镜像 `sys_create_bullet_task_script_wrong_kind_faults_before_creating`。
    #[test]
    fn spawn_enemy_task_script_wrong_kind_faults_without_enemy() {
        let ecl = wrong_kind_task_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        // 正序参：x, y, hp, drop_table, score, sprite, task_script
        let args = [0, 0, 100, 0, 0, 0, 1, 0]; // task_script=1，在册但 CallOnly
        let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(
            w.body.enemies.iter_alive().count(),
            0,
            "先验后建：不留半成品"
        );
    }

    /// P4-b 支路③另一半（终审 M-4）：task 号在册、`kind()==Async`，唯独**带参**
    /// （非零参）→ 同样必须 Fault，且敌未创建。镜像
    /// `sys_create_bullet_task_script_with_params_faults_before_creating`。
    #[test]
    fn spawn_enemy_task_script_with_params_faults_without_enemy() {
        let ecl = async_with_param_task_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        // 正序参：x, y, hp, drop_table, score, sprite, task_script
        let args = [0, 0, 100, 0, 0, 0, 1, 0]; // task_script=1，在册、Async，但带参
        let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(
            w.body.enemies.iter_alive().count(),
            0,
            "先验后建：不留半成品"
        );
    }

    /// 任务池满（P4-a 降级）：敌仍建成、`diag.pool_full[POOL_TASK]` 计数 +1、
    /// `main_task` 保持 0（挂任务失败不影响敌本身创建成功）。
    #[test]
    fn spawn_enemy_task_pool_full_degrades() {
        let ecl = test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("enemy_task", 1)],
            Some(0),
        );
        let mut w = World::new(1);
        // 先灌满任务池（循环 tasks.spawn；owner 值任意，本测试只关心池满信号）。
        while w
            .tasks
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .is_some()
        {}
        let mut task = Task::default();
        let args = [
            Fx::from_int(0).raw(),
            Fx::from_int(80).raw(),
            10,
            0,
            0,
            0,
            1, // 在册但池满
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let handle = task.stack[0];
        assert!(handle >= 0, "任务池满不应阻止敌建成");
        assert_eq!(
            w.body.diag.pool_full[crate::world::POOL_TASK],
            1,
            "任务池满应计一次 P4-a 降级"
        );
        assert_eq!(
            w.body.enemies.main_task[enemy_slot(handle)],
            0,
            "main_task 应保持 0（挂任务失败）"
        );
    }

    /// e2e：敌绑定的 task 随敌死亡被 owner-liveness gate 静默收走（相位 2 门禁，同
    /// `sys_create_bullet_task_script_spawns_owner_bound_task_and_dies_with_bullet` 先例）。
    #[test]
    fn enemy_owned_task_dies_with_enemy() {
        let ecl = test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("enemy_task", 1)],
            Some(0),
        );
        let mut w = World::new(1);
        w.body.frame = 5;
        let mut task = Task::default();
        let args = [
            Fx::from_int(0).raw(),
            Fx::from_int(80).raw(),
            50,
            0,
            0,
            0,
            1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let eidx = enemy_slot(task.stack[0]) as u16;
        let egen = w.body.enemies.generation[eidx as usize];

        let child = (0..crate::ecl::task::TASK_CAP)
            .find(|&i| w.tasks.is_alive(i))
            .expect("应已派生子任务");
        assert!(w.tasks.is_alive(child), "敌死之前任务应活");

        // free 敌 → 次帧任务被静默回收（owner 门禁，通过完整 step 驱动验证端到端链路）。
        w.body.enemies.free(EnemyHandle {
            index: eidx,
            generation: egen,
        });
        let frame = w.body.frame;
        crate::step::step(
            &mut w,
            &TABLES_V0,
            &ecl,
            &crate::input::InputFrame::empty(frame),
        );
        assert!(!w.tasks.is_alive(child), "owner 敌死后任务应被静默回收");
        assert_eq!(w.body.diag.task_faults, 0, "owner 死不是 Fault");
    }

    /// `enemy_hp`（SYS 100）：活敌返当前 hp（判别值 77，非默认）；死敌/越界句柄均返 -1
    /// （P4-b，不 Fault）。
    #[test]
    fn enemy_hp_reads_alive_and_rejects_dead() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 77);
        // `test_support::spawn_enemy` 令 hp==hp_max==77——单独把 hp_max 拉开，逐位命中排除
        // "读混 hp/hp_max 两个同族字段"的变异（同 `sys_self_hp_max_dispatch_by_owner_kind`
        // 先例：hp_max=9999 排除读混字段）。
        w.body.enemies.hp_max[eh.index as usize] = 9999;
        let mut task = Task::default();
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_ENEMY_HP,
                &[crate::enemy::pack_handle(eh)]
            )
            .is_ok()
        );
        assert_eq!(task.stack[0], 77, "活敌返当前 hp（判别值，非 hp_max）");

        w.body.enemies.free(eh);
        task.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_ENEMY_HP,
                &[crate::enemy::pack_handle(eh)]
            )
            .is_ok()
        );
        assert_eq!(task.stack[0], -1, "死敌返 -1");

        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ENEMY_HP, &[9999]).is_ok());
        assert_eq!(task.stack[0], -1, "越界句柄返 -1（P4-b 不 Fault）");
    }

    #[test]
    fn sys_drop_item_creates_item() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = [
            Fx::from_int(1).raw(),
            Fx::from_int(2).raw(),
            crate::items::ITEM_POINT as i32,
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ITEM, &args).is_ok());
        assert!(task.stack[0] >= 0);
        assert_eq!(w.body.items.iter_alive().count(), 1);
    }

    /// self owner=ENEMY 时 `move_enemy_to` 正确武装插值器（`mv_active`/终点/时长/easing）。
    #[test]
    fn sys_move_enemy_to_arms_interpolator() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        // 正序：dur,x,y,easing
        let args = [30, Fx::from_int(50).raw(), Fx::from_int(60).raw(), 2];
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args).is_ok());
        let i = eh.index as usize;
        assert_eq!(w.body.enemies.mv_active[i], 1);
        assert_eq!(w.body.enemies.mv_to_x[i], Fx::from_int(50));
        assert_eq!(w.body.enemies.mv_to_y[i], Fx::from_int(60));
        assert_eq!(w.body.enemies.mv_dur[i], 30);
        assert_eq!(w.body.enemies.mv_easing[i], 2);
    }

    /// D19：`dur`/`easing` 越界 → P4-b 整条 no-op（不是静默截断、也不是 Fault）。
    ///
    /// **旧行为的荒谬正是本条要钉死的**：裸 `as u8` 之下 `easing = 256` 截断成 0、**静默变
    /// Linear** 并照常武装插值器，而 `easing = 264` 落 8、被世界层正确拒掉——能不能拒取决于
    /// 越界值**模 256 落在哪里**。`dur = -1` 同理变成"缓动 65535 帧"。
    ///
    /// **判别力**：`easing = 256` 那条腿是核心——它在旧实现下**会成功武装**（`mv_active == 1`、
    /// `mv_easing == 0`），只有收窄之后才 no-op。若有人把 `try_from` 改回 `as`，或把处置从
    /// no-op 改成钳位，这条立刻红。`dur = 65536`（截断成 0）与 `dur = -1`（截断成 65535）
    /// 两条覆盖 `dur` 的两侧。第四条正例押住"合法值一切照旧"，防"收窄收过头把好参数也拒了"。
    #[test]
    fn move_verbs_reject_out_of_range_dur_and_easing_instead_of_truncating() {
        for (label, args) in [
            // 正序 dur,x,y,easing
            (
                "easing=256（旧实现截断成 0 = Linear，会静默成功）",
                [30, 0, 0, 256],
            ),
            ("easing=-1", [30, 0, 0, -1]),
            ("dur=65536（旧实现截断成 0）", [65536, 0, 0, 2]),
            ("dur=-1（旧实现截断成 65535 ≈ 18 分钟）", [-1, 0, 0, 2]),
        ] {
            let (mut w, ecl) = fresh();
            let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
            let mut task = Task {
                owner_kind: OWNER_ENEMY,
                owner_index: eh.index,
                owner_gen: eh.generation,
                ..Task::default()
            };
            let before = w.body.diag.contract_viol;
            assert!(
                call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args).is_ok(),
                "{label}：坏参数是调用方违约，走 P4-b 不 Fault"
            );
            let i = eh.index as usize;
            assert_eq!(
                w.body.enemies.mv_active[i], 0,
                "{label}：必须整条 no-op，不得武装"
            );
            assert_eq!(w.body.diag.contract_viol - before, 1, "{label}：违约计一次");
            assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS, "{label}");
        }

        // 正例：边界上的合法值照常武装（收窄不得收过头）
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        let args = [65535, Fx::from_int(50).raw(), 0, 7]; // dur 上沿 + easing 上沿(7 = 合法最大)
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args).is_ok());
        let i = eh.index as usize;
        assert_eq!(w.body.enemies.mv_active[i], 1, "边界合法值必须照常武装");
        assert_eq!(w.body.enemies.mv_dur[i], 65535);
        assert_eq!(w.body.enemies.mv_easing[i], 7);
    }

    /// D19：四条速度动词与 `move_enemy_to` **同一条腿**——收窄是共用 helper，五个入口不得分家。
    ///
    /// **判别力**：逐个 syscall 号试 `easing = 256`。若只给 `move_enemy_to` 加了收窄而漏掉
    /// 某条速度动词（本条最可能的实现失误），那一条的 `vel_active` 会是 1，立刻红。
    #[test]
    fn all_five_move_verbs_share_the_same_narrowing_leg() {
        // (号, 正序参数——末位恒为 easing)
        let cases: [(u16, &[i32]); 4] = [
            (SYS_MOVE_VEL, &[30, 0, Fx::from_int(3).raw(), 256]),
            (SYS_MOVE_VEL_XY, &[30, Fx::from_int(3).raw(), 0, 256]),
            (SYS_MOVE_ANGLE, &[30, 0, 256]),
            (SYS_MOVE_SPEED, &[30, Fx::from_int(3).raw(), 256]),
        ];
        for (sys, args) in cases {
            let (mut w, ecl) = fresh();
            let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
            let mut task = Task {
                owner_kind: OWNER_ENEMY,
                owner_index: eh.index,
                owner_gen: eh.generation,
                ..Task::default()
            };
            let before = w.body.diag.contract_viol;
            assert!(
                call(&mut w, &ecl, &mut task, sys, args).is_ok(),
                "syscall {sys}"
            );
            let i = eh.index as usize;
            assert_eq!(
                w.body.enemies.vel_active[i], 0,
                "syscall {sys}：easing=256 必须整条 no-op（收窄漏了这一条入口？）"
            );
            assert_eq!(w.body.diag.contract_viol - before, 1, "syscall {sys}");
        }
    }

    /// self owner != ENEMY → Fault（误用策略：move_enemy_to 要求 self 是敌）。
    #[test]
    fn sys_move_enemy_to_wrong_owner_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        let args = [30, 0, 0, 0];
        let r = call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
    }

    /// move_vel（410）：4 参正序 dur,angle,speed,easing；owner 取自 self。
    #[test]
    fn sys_move_vel_arms_polar_interpolator() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        let args = [
            20,
            crate::math::Angle::QUARTER.raw() as i32,
            Fx::from_int(4).raw(),
            3,
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL, &args).is_ok());
        let i = eh.index as usize;
        assert_eq!(w.body.enemies.vel_active[i], 1);
        assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_POLAR);
        assert_eq!(w.body.enemies.vel_to_0[i], Fx::from_int(4).raw());
        assert_eq!(
            w.body.enemies.vel_to_1[i],
            crate::math::Angle::QUARTER.raw() as i32
        );
        assert_eq!(w.body.enemies.vel_dur[i], 20);
        assert_eq!(w.body.enemies.vel_easing[i], 3);
    }

    /// move_vel_xy（411）：落 CART 空间——**参数序 dur,vx,vy,easing**（vx 在前）。
    #[test]
    fn sys_move_vel_xy_arms_cartesian_interpolator() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        // vx=3, vy=-7：**x≠y 且异号**，两条派发臂写反立刻可辨
        let args = [12, Fx::from_int(3).raw(), Fx::from_int(-7).raw(), 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL_XY, &args).is_ok());
        let i = eh.index as usize;
        assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_CART);
        assert_eq!(
            w.body.enemies.vel_to_0[i],
            Fx::from_int(3).raw(),
            "slot0 = vx"
        );
        assert_eq!(
            w.body.enemies.vel_to_1[i],
            Fx::from_int(-7).raw(),
            "slot1 = vy"
        );
    }

    /// move_angle（420）/ move_speed（421）：3 参，各自保持另一分量。
    #[test]
    fn sys_move_angle_and_speed_are_single_axis() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        let i = eh.index as usize;
        // 先摆一个 speed=7、angle=0 的起点（dur=0 瞬时）
        let seed = [0, 0, Fx::from_int(7).raw(), 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL, &seed).is_ok());
        // move_angle(0, QUARTER, 0) → 只转向
        let a = [0, crate::math::Angle::QUARTER.raw() as i32, 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ANGLE, &a).is_ok());
        assert_eq!(w.body.enemies.speed[i], Fx::from_int(7), "转向不改速率");
        assert_eq!(w.body.enemies.angle[i], crate::math::Angle::QUARTER);
        // move_speed(0, 2.0, 0) → 只调速
        let s = [0, Fx::from_int(2).raw(), 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_SPEED, &s).is_ok());
        assert_eq!(
            w.body.enemies.angle[i],
            crate::math::Angle::QUARTER,
            "调速不改方向"
        );
        assert_eq!(w.body.enemies.speed[i], Fx::from_int(2));
    }

    /// 四条动词 self owner != ENEMY → Fault（同 move_to 现状）。
    #[test]
    fn motion_verbs_fault_on_non_enemy_owner() {
        let (mut w, ecl) = fresh();
        for (sys, argc) in [
            (SYS_MOVE_VEL, 4),
            (SYS_MOVE_VEL_XY, 4),
            (SYS_MOVE_ANGLE, 3),
            (SYS_MOVE_SPEED, 3),
        ] {
            let mut task = Task {
                owner_kind: OWNER_STAGE,
                ..Task::default()
            };
            let args = vec![0i32; argc];
            let r = call(&mut w, &ecl, &mut task, sys, &args);
            assert_eq!(r, Err(FAULT_BAD_OP), "syscall {sys} 非敌 owner 应 Fault");
        }
    }

    /// boss_set：self owner=ENEMY 时 `enemy` 字段自动取自 owner；6 参落槽逐位命中。
    #[test]
    fn sys_boss_set_writes_slot_with_self_owner_enemy() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        // 正序：slot,hp_ratio,spell_id,timer_frames,phase_left,active
        let args = [0, Fx::from_raw(32768).raw(), 7, 3600, 2, 1];
        assert!(call(&mut w, &ecl, &mut task, SYS_BOSS_SET, &args).is_ok());
        let ui = w.body.boss_ui[0];
        assert_eq!(ui.enemy, eh);
        assert_eq!(ui.hp_ratio, Fx::from_raw(32768));
        assert_eq!(ui.spell_id, 7);
        assert_eq!(ui.timer_frames, 3600);
        assert_eq!(ui.phase_left, 2);
        assert_eq!(ui.active, 1);
    }

    /// boss_set：self owner 非 ENEMY → `enemy` 字段写 NULL，不 Fault（例外，见模块文档）。
    #[test]
    fn sys_boss_set_non_enemy_owner_writes_null_enemy_no_fault() {
        let (mut w, ecl) = fresh();
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        let args = [0, 0, 0, 0, 0, 1];
        assert!(call(&mut w, &ecl, &mut task, SYS_BOSS_SET, &args).is_ok());
        assert_eq!(w.body.boss_ui[0].enemy, EnemyHandle::NULL);
        assert_eq!(w.body.boss_ui[0].active, 1);
    }

    #[test]
    fn sys_pulse_signal_sets_channel() {
        let (mut w, ecl) = fresh();
        w.body.frame = 10;
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_PULSE_SIGNAL, &[3]).is_ok());
        assert_eq!(w.body.signals[3], 11, "脉冲戳为 frame+1（边沿语义）");
    }

    /// 弹 setter 族快乐路径抽查（self owner=BULLET）：speed/vel/ang_vel/gravity/stop_fx。
    #[test]
    fn sys_bullet_setter_family_happy_path() {
        let (mut w, ecl) = fresh();
        let bh = crate::world::test_support::bullet_at(&mut w, 0, 100);
        let mut task = Task {
            owner_kind: OWNER_BULLET,
            owner_index: bh.index,
            owner_gen: bh.generation,
            ..Task::default()
        };
        let i = bh.index as usize;

        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SET_BULLET_SPEED,
                &[Fx::from_int(3).raw()]
            )
            .is_ok()
        );
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3));

        task.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SET_BULLET_VEL,
                &[Fx::from_int(1).raw(), Fx::from_int(2).raw()]
            )
            .is_ok(),
            "正序 vx,vy"
        );
        assert_eq!(w.body.bullets.vx[i], Fx::from_int(1));
        assert_eq!(w.body.bullets.vy[i], Fx::from_int(2));

        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_BULLET_ANG_VEL, &[300]).is_ok());
        assert_eq!(w.body.bullets.ang_vel[i], 300);

        task.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SET_BULLET_GRAVITY,
                &[Fx::from_raw(11).raw(), Fx::from_raw(22).raw()]
            )
            .is_ok(),
            "正序 ax,ay"
        );
        assert_eq!(w.body.bullets.ax[i].raw(), 11);
        assert_eq!(w.body.bullets.ay[i].raw(), 22);

        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_STOP_BULLET_FX, &[]).is_ok());
        assert_eq!(
            w.body.bullets.flags[i]
                & (crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX),
            0
        );
    }

    /// 弹 setter 族误用策略：self owner != BULLET → Fault（响亮报错，非静默 no-op）。
    /// **B15 覆盖缺口补齐**：九支 dispatch 分支各自手写内联 `self_bullet_handle(task)?`
    /// （非共享 loop/宏），此前只有 `SYS_SET_BULLET_SPEED` 一支独立实测过——其余 8 支
    /// 理论上"该有这一句"，但没有测试证明真的有；有人加新 setter 或手滑漏抄这一句都
    /// 不会被任何测试发现。一刀补齐全九支：`self_bullet_handle` 在摸任何参数前就返回
    /// Fault，故 `&[]` 空参足够触发，不依赖各 syscall 各自的真实 arity。
    #[test]
    fn sys_bullet_setter_wrong_owner_faults() {
        const ALL_NINE: [(u16, &str); 9] = [
            (SYS_SET_BULLET_SPEED, "set_speed"),
            (SYS_SET_BULLET_ANGLE, "set_angle"),
            (SYS_TURN_BULLET, "turn"),
            (SYS_SET_BULLET_VEL, "set_vel"),
            (SYS_SET_BULLET_ANG_VEL, "set_ang_vel"),
            (SYS_SET_BULLET_ACCEL, "set_accel"),
            (SYS_SET_BULLET_GRAVITY, "set_gravity"),
            (SYS_STOP_BULLET_FX, "stop_fx"),
            (SYS_AIM_BULLET_AT_PLAYER, "aim_at_player"),
        ];
        for (sys, name) in ALL_NINE {
            let (mut w, ecl) = fresh();
            let mut task = Task {
                owner_kind: OWNER_ENEMY,
                ..Task::default()
            };
            let r = call(&mut w, &ecl, &mut task, sys, &[]);
            assert_eq!(
                r,
                Err(FAULT_BAD_OP),
                "'{name}'（sys={sys}）坏 owner 应 Fault"
            );
        }
    }

    /// self 位置朝向 P0 的角度 == 参考 `atan2` 计算（判别式：换公式即红）。
    #[test]
    fn sys_aim_player_angle_matches_atan2_reference() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 100, 100, 5);
        // 自机出场点 (0, 384)（`World::new` 既定）。
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_AIM_PLAYER_ANGLE, &[]).is_ok());
        let expect = crate::math::cordic::atan2(Fx::from_int(384 - 100), Fx::from_int(0 - 100));
        assert_eq!(task.stack[0] as u16 as u32, expect.raw() as u32);
    }

    /// 判别式（F8 瞄准口径统一）：`aim_player()` 取的是**最近的可瞄自机**，不是恒
    /// `players[0]`。P0 已 GAMEOVER 冻在左上、P1 存活在右下——两个方向截然相反，取错
    /// 人一眼可辨。摆位不可重合：圆心重合式的摆法对"瞄谁"这条映射是瞎的
    /// （`CLAUDE.md` 点名的 M0-7 变异检验教训）。
    #[test]
    fn sys_aim_player_angle_skips_unaimable_player() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        w.body.players[0].x = Fx::from_int(-200);
        w.body.players[0].y = Fx::from_int(-150);
        w.body.players[0].life_state = crate::player::LIFE_GAMEOVER;
        w.body.players[1] = crate::player::PlayerState::spawn(0, &TABLES_V0.characters[0]);
        w.body.players[1].x = Fx::from_int(200);
        w.body.players[1].y = Fx::from_int(150);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_AIM_PLAYER_ANGLE, &[]).is_ok());
        let to_p1 = crate::math::cordic::atan2(Fx::from_int(150), Fx::from_int(200));
        let to_p0 = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
        assert_eq!(
            task.stack[0] as u16 as u32,
            to_p1.raw() as u32,
            "应瞄唯一可瞄的 P1"
        );
        assert_ne!(
            task.stack[0] as u16 as u32,
            to_p0.raw() as u32,
            "瞄了已 GAMEOVER 的 P0 尸体坐标"
        );
    }

    /// 一个可瞄自机都没有时的 fallback 口径（F8 拍板，本条押的是"别改成别的"）：查询与
    /// 发射这两条**必须产出一个角度**的路回退到 `players[0]` 的最后坐标——不是朝下的
    /// 约定角、不是 0、更不是 no-op。no-op 是弹上 setter 那半边的处置，两半有意不同：
    /// 查询必须给值，弹上的 setter 可以拒绝改动。
    #[test]
    fn sys_aim_player_angle_falls_back_to_p0_when_none_aimable() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        w.body.players[0].x = Fx::from_int(-200);
        w.body.players[0].y = Fx::from_int(-150);
        w.body.players[0].life_state = crate::player::LIFE_GAMEOVER; // players[1] 本就 ABSENT
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_AIM_PLAYER_ANGLE, &[]).is_ok());
        let to_corpse = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
        assert_eq!(
            task.stack[0] as u16 as u32,
            to_corpse.raw() as u32,
            "无可瞄自机时应回退到 P0 的最后坐标"
        );
    }

    /// 坏 syscall 号（不在 v1 号表内）→ Fault（`dispatch` 默认臂，复用 `FAULT_BAD_OP`）。
    #[test]
    fn dispatch_bad_syscall_number_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert_eq!(call(&mut w, &ecl, &mut task, 9999, &[]), Err(FAULT_BAD_OP));
    }

    /// [`syscall_implemented`] 的白名单必须与冻结号表**逐条等同**——它是跨 crate 的唯一
    /// 出口（`dispatch` 是 `pub(crate)`、[`frozen_table`] 是 `cfg(test)`，编译器 crate
    /// 两个都够不着），一旦与真实派发面漂移，`stg-ecl-compiler` 那条 `is_op` 一致性测试
    /// 就会拿错判据、变成假绿。
    ///
    /// 判据是**全 `u16` 域扫一遍**（65536 次 `matches!`，测试里无所谓），不是"照表逐条问
    /// 一遍"——后者只能抓"漏了一条"，抓不到"多写了一条表里没有的号"。两个方向都要：
    /// 加 syscall 忘了加进白名单 → 数量对不上；白名单里留了一条已删的号 → 同样红。
    #[test]
    fn syscall_whitelist_matches_the_frozen_table() {
        let table = frozen_table();

        // 方向一：表里每条都必须在白名单内。
        for &(num, name, _) in table {
            assert!(
                syscall_implemented(num),
                "{name}(={num}) 在冻结号表里却不在 syscall_implemented 白名单内"
            );
        }

        // 方向二：全域里为真的号，恰好只有表里那些（抓"多写了一条"）。
        let live: Vec<u16> = (0..=u16::MAX).filter(|&n| syscall_implemented(n)).collect();
        assert_eq!(
            live.len(),
            table.len(),
            "白名单为真的号有 {} 个，冻结号表却是 {} 条",
            live.len(),
            table.len()
        );
        for n in &live {
            assert!(
                table.iter().any(|&(num, _, _)| num == *n),
                "白名单里的 {n} 不在冻结号表内（多写/该删未删）"
            );
        }

        // 方向三：把白名单钉到 `dispatch` 的兜底臂上——白名单说没有的号，`dispatch`
        // 必须以 `FAULT_BAD_OP` 拒绝（抽查，不做全域派发：派发有副作用）。
        let (mut w, ecl) = fresh();
        for probe in [9999u16, 33, 99, 104, 199, 899, u16::MAX] {
            assert!(!syscall_implemented(probe), "{probe} 不该在白名单里");
            let mut task = Task::default();
            assert_eq!(
                call(&mut w, &ecl, &mut task, probe, &[]),
                Err(FAULT_BAD_OP),
                "白名单外的 {probe} 应被 dispatch 兜底臂拒绝"
            );
        }
    }

    /// 栈下溢（参数不足）→ `Fault(FAULT_STACK)`：`SET_VAR` 需 2 参，空栈直接派发。
    #[test]
    fn dispatch_stack_underflow_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[]),
            Err(FAULT_STACK)
        );
    }

    #[test]
    fn sys_stage_clear_pushes_fact_event_not_request() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_STAGE_CLEAR, &[3]).is_ok());
        let evs: Vec<_> = w
            .frame_events()
            .iter()
            .filter(|e| e.kind == crate::events::EVT_STAGE_CLEARED)
            .collect();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data[0], 3, "载荷 = stage");
        assert_eq!(
            w.body.take_requests().len(),
            0,
            "走通道 A 事实流，不发通道 B 请求"
        );
        assert_eq!(task.sp, 0, "参数弹尽");
    }

    #[test]
    fn sys_emit_req_pushes_request_and_drains_stack() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_EMIT_REQ,
                &[64, 1, 2, 3, 4, 5, 6]
            )
            .is_ok()
        );
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!((reqs[0].id, reqs[0].seq), (64, 0));
        assert_eq!(
            reqs[0].args,
            [1, 2, 3, 4, 5, 6],
            "声明序 id,a0..a5 ↔ 弹栈逆序还原"
        );
        assert_eq!(task.sp, 0, "七值全弹栈");
    }

    #[test]
    fn sys_emit_req_bad_id_is_p4b_noop() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let cv0 = w.body.diag.contract_viol;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_EMIT_REQ,
                &[-1, 0, 0, 0, 0, 0, 0]
            )
            .is_ok()
        );
        assert_eq!(w.body.take_requests().len(), 0, "坏 id no-op 不入缓冲");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(
            task.sp, 0,
            "坏 id 路径同样七值全弹栈（先弹后验，栈效应与成功路径一致）"
        );
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_EMIT_REQ,
                &[65536, 0, 0, 0, 0, 0, 0]
            )
            .is_ok()
        );
        assert_eq!(w.body.take_requests().len(), 0, "越上界同款");
        assert_eq!(w.body.diag.contract_viol, cv0 + 2);
        assert_eq!(
            task.sp, 0,
            "坏 id 路径同样七值全弹栈（先弹后验，栈效应与成功路径一致）"
        );
    }

    #[test]
    fn sys_emit_req_stack_underflow_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_EMIT_REQ, &[1, 2]),
            Err(FAULT_STACK),
            "参数不足 → 栈下溢 Fault（同全族处置）"
        );
    }

    // ── SYS_SPELL_BEGIN/END/TIMER（Task 2；符卡机构 spec 2026-07-24 §5）─────────

    fn enemy_task(w: &mut World, hp: i32) -> (EnemyHandle, Task) {
        let boss = crate::world::test_support::spawn_enemy(w, 0, 80, hp);
        let task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: boss.index,
            owner_gen: boss.generation,
            ..Task::default()
        };
        (boss, task)
    }

    /// misuse 策略：owner != ENEMY → Fault，零副作用（同 `move_enemy_to` 口径）。
    #[test]
    fn sys_spell_begin_owner_stage_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        let args = [0, 1, -1, 100, 1000, 0, 0];
        let r = call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.spells[0].active, 0, "misuse Fault：零副作用");
    }

    /// 快乐路径：owner=ENEMY 成功 → 槽 active + 模式任务已 spawn，`spell_bound==slot+1`，
    /// owner 绑定新任务到 boss（"生"，spec §2.1）。
    #[test]
    fn sys_spell_begin_success_spawns_pattern_task_bound_to_slot() {
        let mut w = World::new(1);
        let ecl = async_pattern_image();
        let (boss, mut task) = enemy_task(&mut w, 1000);
        // 声明序：slot, spell_id, pattern_ref, time_limit, bonus0, flags, threshold
        let args = [0, 5, 1, 100, 1000, 0, 300];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.spells[0].active, 1, "槽应已 active");
        let child = (0..crate::ecl::task::TASK_CAP)
            .find(|&i| w.tasks.is_alive(i))
            .expect("应已 spawn 模式任务");
        assert_eq!(w.tasks.slots[child].owner_kind, OWNER_ENEMY);
        assert_eq!(w.tasks.slots[child].owner_index, boss.index);
        assert_eq!(w.tasks.slots[child].owner_gen, boss.generation);
        assert_eq!(
            w.tasks.slots[child].spell_bound, 1,
            "槽 0 → spell_bound = slot+1 = 1"
        );
        assert_eq!(
            w.tasks.slots[child].spell_epoch, w.body.spells[0].epoch,
            "模式任务应捕获本槽刚铸出的代际戳（ABA 修复，复审 Task 2）"
        );
        assert_ne!(w.body.spells[0].epoch, 0, "首次 begin 后 epoch 应已非零");
    }

    /// `pattern=none`（负值）→ begin 仍成功但不 spawn 任何任务。
    #[test]
    fn sys_spell_begin_pattern_none_does_not_spawn() {
        let mut w = World::new(1);
        let ecl = async_pattern_image();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let args = [0, 5, -1, 100, 1000, 0, 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.spells[0].active, 1);
        assert_eq!(w.tasks.iter_alive().count(), 0, "pattern=none 不 spawn");
    }

    /// 坏 `pattern` 号（不在册）→ Fault，先查后建：`spell_begin_internal` 不应已被调用
    /// （槽仍空闲，同 `sys_create_bullet` 的坏 task_script 口径）。
    #[test]
    fn sys_spell_begin_pattern_bad_sub_faults_before_mutating_world() {
        let mut w = World::new(1);
        let ecl = async_pattern_image(); // 只有 raw=1 在册
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let args = [0, 5, 99, 100, 1000, 0, 0]; // pattern_ref=99 不在册
        let r = call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(w.body.spells[0].active, 0, "先查后建：坏号不应已写入槽");
        assert_eq!(w.tasks.iter_alive().count(), 0);
    }

    /// 槽越界 → P4-b no-op + 计数（`spell_begin_internal` 自身判定，syscall 层直通）。
    #[test]
    fn sys_spell_begin_slot_oob_is_p4b_noop() {
        let (mut w, ecl) = fresh();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let cv0 = w.body.diag.contract_viol;
        let args = [99, 5, -1, 100, 1000, 0, 0]; // slot=99 越界
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.tasks.iter_alive().count(), 0);
    }

    /// `threshold > 当前 hp` → P4-b no-op + 计数。
    #[test]
    fn sys_spell_begin_threshold_over_hp_is_p4b_noop() {
        let (mut w, ecl) = fresh();
        let (_boss, mut task) = enemy_task(&mut w, 100); // hp=100
        let cv0 = w.body.diag.contract_viol;
        let args = [0, 5, -1, 100, 1000, 0, 200]; // threshold=200 > hp=100
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.spells[0].active, 0);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    }

    /// 该槽已 active（重复 begin 同一槽）→ P4-b no-op + 计数，原槽内容不被覆写。
    #[test]
    fn sys_spell_begin_duplicate_active_slot_is_p4b_noop() {
        let (mut w, ecl) = fresh();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let args = [0, 5, -1, 100, 1000, 0, 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.spells[0].active, 1);

        task.sp = 0;
        let cv0 = w.body.diag.contract_viol;
        let args2 = [0, 6, -1, 100, 1000, 0, 0]; // 同槽再次 begin
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args2).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.spells[0].spell_id, 5, "第一次的槽内容未被覆写");
    }

    /// 控制器补充决策（复审 T1-m3）：`threshold < 0` → P4-b no-op + 计数，不 begin、不
    /// spawn（血线语义非负；世界 API 层仍宽松接受负值供白盒测）。
    #[test]
    fn sys_spell_begin_negative_threshold_is_p4b_noop_no_spawn() {
        let mut w = World::new(1);
        let ecl = async_pattern_image();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let cv0 = w.body.diag.contract_viol;
        let args = [0, 5, 1, 100, 1000, 0, -1]; // threshold=-1，pattern 合法在册
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.spells[0].active, 0, "拒负 threshold：不 begin");
        assert_eq!(w.tasks.iter_alive().count(), 0, "不 spawn");
    }

    /// 控制器补充决策（Task 2 复审 Important #2，spec §5 P4-b 拒收条件）：`bonus0 < 0` →
    /// P4-b no-op + 计数，不 begin、不 spawn——必须在 `bonus0 as u32` 转型前拒收，否则
    /// `-1i32 as u32` 变 `u32::MAX`，等于凭空发 ~42.9 亿分（未拒时的真实后果）。
    #[test]
    fn sys_spell_begin_negative_bonus0_is_p4b_noop_no_spawn() {
        let mut w = World::new(1);
        let ecl = async_pattern_image();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let cv0 = w.body.diag.contract_viol;
        let args = [0, 5, 1, 100, -1, 0, 0]; // bonus0=-1，pattern 合法在册
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.spells[0].active, 0, "拒负 bonus0：不 begin");
        assert_eq!(w.tasks.iter_alive().count(), 0, "不 spawn");
    }

    /// 控制器补充决策（Task 2 复审 Important #3，spec §5 P4-b 拒收条件）：`time_limit <= 0`
    /// → P4-b no-op + 计数，不 begin、不 spawn——负值经 `as u16` 抹符号会变成巨大正数
    /// （如 `-1 → 65535`），绕过 `spell_begin_internal` 内"`time_limit == 0`"那条守卫，
    /// 未拒时会带着离谱的时限成功 begin。
    #[test]
    fn sys_spell_begin_non_positive_time_limit_is_p4b_noop_no_spawn() {
        let mut w = World::new(1);
        let ecl = async_pattern_image();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let cv0 = w.body.diag.contract_viol;
        let args = [0, 5, 1, -1, 1000, 0, 0]; // time_limit=-1，pattern 合法在册
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.spells[0].active, 0, "拒非正 time_limit：不 begin");
        assert_eq!(w.tasks.iter_alive().count(), 0, "不 spawn");

        // 对照：time_limit=0 同样应被拒（非仅负值——`<= 0` 覆盖两态）。
        task.sp = 0;
        let cv1 = w.body.diag.contract_viol;
        let args0 = [0, 5, 1, 0, 1000, 0, 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args0).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv1 + 1);
        assert_eq!(w.body.spells[0].active, 0, "time_limit==0 同样拒收");
    }

    /// 控制器补充决策（复审 T1-m1）：该 boss 已绑到另一 active 槽 → P4-b no-op + 计数
    /// （单卡/boss 是设计用法；同 boss 连续 begin 两个不同槽应在第二次被拒）。
    #[test]
    fn sys_spell_begin_boss_already_bound_to_another_slot_is_p4b_noop() {
        let (mut w, ecl) = fresh();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let args0 = [0, 5, -1, 100, 1000, 0, 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args0).is_ok());
        assert_eq!(w.body.spells[0].active, 1);

        task.sp = 0;
        let cv0 = w.body.diag.contract_viol;
        let args1 = [1, 6, -1, 100, 1000, 0, 0]; // 同 boss，另一空闲槽 1
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args1).is_ok());
        assert_eq!(
            w.body.diag.contract_viol,
            cv0 + 1,
            "同 boss 二次绑定应计数拒绝"
        );
        assert_eq!(w.body.spells[1].active, 0, "槽 1 不应被写入");
    }

    /// 逃生舱口：绑定 owner 调用 → 槽走 HP 路径结算清空。
    #[test]
    fn sys_spell_end_bound_owner_settles_slot() {
        let (mut w, ecl) = fresh();
        let (boss, mut task) = enemy_task(&mut w, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 5, 100, 1000, 0, 0));
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_END, &[]).is_ok());
        assert_eq!(w.body.spells[0].active, 0, "逃生舱口应已结算清槽");
    }

    /// 无绑定 → no-op，且不计 contract_viol（重复调用安全，同世界 API 文档）。
    #[test]
    fn sys_spell_end_unbound_owner_is_noop() {
        let (mut w, ecl) = fresh();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        let cv0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_END, &[]).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0, "无绑定重复调用安全，不计数");
    }

    /// 绑定槽 → 压 `frames_left`（begin 当刻即 `time_limit`）。
    #[test]
    fn sys_spell_timer_bound_pushes_frames_left() {
        let (mut w, ecl) = fresh();
        let (boss, mut task) = enemy_task(&mut w, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 5, 100, 1000, 0, 0));
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_TIMER, &[]).is_ok());
        assert_eq!(task.stack[0], 100);
    }

    /// 无绑定（owner=ENEMY 但从未 begin）→ 押 -1。
    #[test]
    fn sys_spell_timer_unbound_pushes_neg1() {
        let (mut w, ecl) = fresh();
        let (_boss, mut task) = enemy_task(&mut w, 1000);
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_TIMER, &[]).is_ok());
        assert_eq!(task.stack[0], -1);
    }

    /// 读族误用降级（同 `SYS_SELF_HP`）：owner 非 ENEMY → 押 -1，不 Fault。
    #[test]
    fn sys_spell_timer_non_enemy_owner_pushes_neg1_no_fault() {
        let (mut w, ecl) = fresh();
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_TIMER, &[]).is_ok());
        assert_eq!(task.stack[0], -1);
    }

    // ── SYS_ADD_SCORE/SYS_BGM/SYS_BG/SYS_BG_PHASE（整局流程刀 Task 2；5xx 族）────────

    #[test]
    fn sys_bgm_writes_field_and_emits_req() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_BGM, &[5]).is_ok());
        assert_eq!(w.body.bgm_id, 5);
        let reqs = w.body.take_requests();
        let last = reqs.last().expect("SYS_BGM 必须发一条 req");
        assert_eq!(last.id, crate::consts::REQ_BGM);
        assert_eq!(last.args[0], 5);
    }

    #[test]
    fn sys_bg_phase_stamps_current_frame() {
        let (mut w, ecl) = fresh();
        w.body.frame = 42;
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_BG_PHASE, &[3]).is_ok());
        assert_eq!(w.body.bg_phase, 3);
        assert_eq!(w.body.bg_phase_frame, 42);
        let reqs = w.body.take_requests();
        let last = reqs.last().expect("SYS_BG_PHASE 必须发一条 req");
        assert_eq!(last.id, crate::consts::REQ_BG_PHASE);
    }

    /// 越界 → P4-b no-op + viol（与 `sys_emit_req` 的 id 收窄同款口径，syscall.rs:622 参照）。
    #[test]
    fn sys_anchor_out_of_range_is_noop_with_viol() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let cv0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut task, SYS_BGM, &[-1]).is_ok());
        assert_eq!(w.body.bgm_id, 0, "字段不动");
        assert_eq!(w.body.take_requests().len(), 0, "无 req");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }

    #[test]
    fn sys_add_score_saturates_at_zero() {
        let (mut w, ecl) = fresh();
        w.body.players[0].score = 10;
        let mut task = Task::default();

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_SCORE, &[-100]).is_ok());
        assert_eq!(w.body.players[0].score, 0, "下溢钳 0，不回绕");

        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_SCORE, &[7]).is_ok());
        assert_eq!(w.body.players[0].score, 7);

        w.body.players[0].score = u64::MAX - 1;
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_SCORE, &[100]).is_ok());
        assert_eq!(w.body.players[0].score, u64::MAX, "上溢钳 u64::MAX");
    }

    // ── SYS_TIME_STOP_PLAYER（560；自机能力刀 T6）───────────────────────────

    /// 写 freeze_left[1]（冻 A+B），不碰 [0]。
    #[test]
    fn sys_time_stop_player_writes_the_cutscene_slot() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[90]).is_ok());
        assert_eq!(w.body.freeze_left, [0, 90]);
    }

    /// 重入取**覆盖**（后写为准），不取最大、不叠加：脚本是权威，覆盖可预测。
    /// 判别力：先写大值再写小值——取最大或叠加的实现在这里会给出 ≥ 90 的值。
    #[test]
    fn sys_time_stop_player_reentry_overwrites() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[90]).is_ok());
        assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[10]).is_ok());
        assert_eq!(w.body.freeze_left[1], 10, "后写为准");
    }

    /// `time_stop_player(0)` = **立即解除**，天然的取消 API（不另开 syscall）。
    #[test]
    fn sys_time_stop_player_zero_cancels() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.freeze_left[1] = 50;
        assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[0]).is_ok());
        assert_eq!(w.body.freeze_left[1], 0);
    }

    /// 参数收窄照 D19 判例：`u16::try_from` 失败 ⇒ P4-b **整条 no-op**、计 contract_viol
    /// 与 BAD_ARGS，**不钳位、不 Fault**。判别力：负值与 65536 各一腿——钳位实现会给出
    /// 0 或 65535 而非"原值不动"。
    #[test]
    fn sys_time_stop_player_rejects_out_of_range() {
        for bad in [-1i32, 65536] {
            let (mut w, ecl) = fresh();
            let mut task = Task::default();
            w.body.freeze_left[1] = 7;
            let cv0 = w.body.diag.contract_viol;
            assert!(
                call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[bad]).is_ok(),
                "P4-b 是安全结果不是 Fault"
            );
            assert_eq!(
                w.body.freeze_left[1], 7,
                "越界 {bad} 必须整条 no-op（不钳位）"
            );
            assert_eq!(
                w.body.diag.contract_viol,
                cv0 + 1,
                "越界 {bad} 须计一次违约"
            );
            assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        }
    }

    // ── SYS_CLEAR_BULLETS（B19；整局流程刀 Task 2）──────────────────────────

    /// B19：`clear_bullets()` 铺一个覆盖全场、存活 1 帧的消弹区。
    /// **消弹转星星是白送的**（M0-15：settle 趟一对每颗被消的弹原位转一颗星星）,
    /// 故断言"弹没了"**和**"星星出现了"——后者是"真走了 FieldPool 那条路"的判别腿
    /// （若实现者绕开 field、自己写个循环把弹 free 掉，星星那条立刻红）。
    #[test]
    fn clear_bullets_lays_fullscreen_field_and_converts_to_stars() {
        use crate::bullets::BULLET_CLEARED;
        use crate::events::EVT_FIELD_CLEARED;
        use crate::items::ITEM_STAR;
        #[cfg(debug_assertions)]
        use crate::world::PH_COLLIDE;
        use crate::world::test_support::bullet_at;

        let (mut w, ecl) = fresh();
        // 三颗弹散在场内不同位置（场界 x∈[-192,192]、y∈[0,448]），全部落在
        // FIELD_RADIUS_FULLSCREEN 的覆盖范围内。
        bullet_at(&mut w, -100, 50);
        bullet_at(&mut w, 100, 400);
        bullet_at(&mut w, 0, 224);

        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_CLEAR_BULLETS, &[]).is_ok());

        // 手动驱动到相位 6/7：field 是 life=1，相位5 减到 0、相位6 alive 位仍在
        // 照常判定（field.rs 模块文档），故 collide 仍能吃到它。
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&TABLES_V0);
        w.body.settle(&TABLES_V0);

        assert_ne!(w.body.bullets.flags[0] & BULLET_CLEARED, 0, "弹0 应被消");
        assert_ne!(w.body.bullets.flags[1] & BULLET_CLEARED, 0, "弹1 应被消");
        assert_ne!(w.body.bullets.flags[2] & BULLET_CLEARED, 0, "弹2 应被消");

        // 判别腿：星星必须出现（真走了 FieldPool），不是"弹没了"就算数。
        assert_eq!(
            w.body.items.iter_alive().count(),
            3,
            "每颗被消的弹应原位转一颗星星"
        );
        for i in 0..3 {
            assert_eq!(w.body.items.item_type[i], ITEM_STAR, "槽 {i} 应为星星");
        }

        assert_eq!(w.body.frame_events_len, 1, "field 消弹应聚合发一条事件");
        assert_eq!(w.body.frame_events[0].kind, EVT_FIELD_CLEARED);
        assert_eq!(w.body.frame_events[0].data[0], 3, "data[0] == 弹数");
    }

    /// P4-a：field 池满 → 确定性降级（不 panic、不 Fault，计 pool_full[POOL_FIELD]）。
    #[test]
    fn clear_bullets_field_pool_full_degrades() {
        use crate::field::{FieldHandle, FieldInit, FieldPool};
        use crate::world::POOL_FIELD;

        let (mut w, ecl) = fresh();
        for _ in 0..FieldPool::CAP {
            let h = w.body.create_field(FieldInit {
                x: Fx::ZERO,
                y: Fx::ZERO,
                radius: Fx::from_int(10),
                dmg_per_frame: 0,
                life: 1,
                owner: 0,
                flags: 0,
            });
            assert_ne!(h, FieldHandle::NULL, "灌池阶段不该失败");
        }
        let before = w.body.diag.pool_full[POOL_FIELD];

        let mut task = Task::default();
        let result = call(&mut w, &ecl, &mut task, SYS_CLEAR_BULLETS, &[]);

        assert!(result.is_ok(), "池满降级不应 Fault");
        assert_eq!(w.body.diag.pool_full[POOL_FIELD], before + 1, "池满须计数");
    }

    // ── SYS_ADD_LIVES/SYS_ADD_BOMBS/SYS_ADD_POWER（B20；账面增量 setter）──────────

    /// B20：`add_lives(d)` 增量记账，双边钳位（P4-b），不回绕不 panic。
    #[test]
    fn add_lives_clamps_both_ends() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.players[0].lives = 3;

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[1]).is_ok());
        assert_eq!(w.body.players[0].lives, 4, "正增");

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[-1]).is_ok());
        assert_eq!(w.body.players[0].lives, 3, "负减");

        w.body.players[0].lives = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[-1]).is_ok());
        assert_eq!(w.body.players[0].lives, 0, "下钳 0，不回绕成 255");

        w.body.players[0].lives = u8::MAX;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[1]).is_ok());
        assert_eq!(w.body.players[0].lives, u8::MAX, "上钳 u8::MAX，不回绕成 0");
    }

    /// `add_bombs(d)`：钳 `[0, STOP_STOCK_MAX=5]`（玩法刀；原 `[0,255]`）。判别腿：写错字段会让 lives 动。
    #[test]
    fn add_bombs_clamps_both_ends() {
        use crate::player::STOP_STOCK_MAX;
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.players[0].bombs = 3;
        let lives0 = w.body.players[0].lives;

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[1]).is_ok());
        assert_eq!(w.body.players[0].bombs, 4, "正增");
        assert_eq!(w.body.players[0].lives, lives0, "判别腿：不得误写 lives");

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[-1]).is_ok());
        assert_eq!(w.body.players[0].bombs, 3, "负减");

        w.body.players[0].bombs = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[-1]).is_ok());
        assert_eq!(w.body.players[0].bombs, 0, "下钳 0，不回绕成 255");

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[9]).is_ok());
        assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "上钳 5");
    }

    /// `power` 上限是 `POWER_MAX`（400，= 显示 4.00），**不是 `u16::MAX`**——判别腿：
    /// 钳错成 `u16::MAX` 时 401 会被放行，`power_tier` 索引随后 OOB（见 `set_player_power` 文档）。
    #[test]
    fn add_power_clamps_to_power_max_not_u16_max() {
        use crate::items::POWER_MAX;

        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.players[0].power = 100;

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[50]).is_ok());
        assert_eq!(w.body.players[0].power, 150, "正增");

        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[-50]).is_ok());
        assert_eq!(w.body.players[0].power, 100, "负减");

        w.body.players[0].power = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[-1]).is_ok());
        assert_eq!(w.body.players[0].power, 0, "下钳 0");

        w.body.players[0].power = POWER_MAX;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[1]).is_ok());
        assert_eq!(
            w.body.players[0].power, POWER_MAX,
            "上钳 POWER_MAX(400)，不是 u16::MAX"
        );

        // 一步跨过上限也得钳住（不是"只在恰好 +1 时钳"）。
        w.body.players[0].power = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[10_000]).is_ok());
        assert_eq!(w.body.players[0].power, POWER_MAX, "一步跨越同样钳 400");
    }

    /// P4-b：脚本可以 push 任意 `i32`，极值 delta 不得让中间量溢出（debug 下裸 `+` 会 panic）。
    /// 三个 setter × `{i32::MAX, i32::MIN}` 全走一遍：不 panic、不 Fault，且落在各自钳位边界上。
    #[test]
    fn add_counters_survive_extreme_deltas() {
        use crate::items::POWER_MAX;

        let (mut w, ecl) = fresh();
        let mut task = Task::default();

        for &no in &[SYS_ADD_LIVES, SYS_ADD_BOMBS] {
            w.body.players[0].lives = 200;
            w.body.players[0].bombs = 200;
            assert!(call(&mut w, &ecl, &mut task, no, &[i32::MAX]).is_ok());
            let v = if no == SYS_ADD_LIVES {
                w.body.players[0].lives
            } else {
                w.body.players[0].bombs
            };
            let cap = if no == SYS_ADD_LIVES {
                u8::MAX
            } else {
                crate::player::STOP_STOCK_MAX
            };
            assert_eq!(v, cap, "syscall {no}：i32::MAX 应钳到上限");

            assert!(call(&mut w, &ecl, &mut task, no, &[i32::MIN]).is_ok());
            let v = if no == SYS_ADD_LIVES {
                w.body.players[0].lives
            } else {
                w.body.players[0].bombs
            };
            assert_eq!(v, 0, "syscall {no}：i32::MIN 应钳到 0");
        }

        w.body.players[0].power = 200;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[i32::MAX]).is_ok());
        assert_eq!(
            w.body.players[0].power, POWER_MAX,
            "i32::MAX 应钳到 POWER_MAX"
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[i32::MIN]).is_ok());
        assert_eq!(w.body.players[0].power, 0, "i32::MIN 应钳到 0");
    }

    // ── 敌人死亡效果四 syscall（520-530；敌人死亡效果刀 T3）────────────────────

    /// 造一只 owner 敌 + 指向它的任务（掉落计数**空**——要灌表 1 用 `load_drop_table_1`）。
    fn enemy_owner_task(w: &mut World, hp: i32, score: u16) -> (EnemyHandle, Task) {
        let eh = crate::world::test_support::spawn_enemy(w, 0, 0, hp);
        w.body.enemies.score[eh.index as usize] = score;
        let task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        (eh, task)
    }

    /// 把内建掉落表 1 展开（`ITEM_POWER`×2 + `ITEM_POINT`×1 = **3 颗**）灌进敌的掉落计数。
    fn load_drop_table_1(w: &mut World, eh: EnemyHandle) {
        let (counts, ok) = crate::tables::drop_counts(&TABLES_V0, 1);
        assert!(ok, "内建掉落表 1 必须存在");
        assert_eq!(
            counts.iter().map(|&n| n as u32).sum::<u32>(),
            3,
            "本组测试的 3 颗判别值依赖表 1 的内容"
        );
        w.body.enemies.drop_count[eh.index as usize] = counts;
    }

    /// D-3 判别腿：`drop_items()` 吐完**不清空**计数 → 再死一次会掉**双份**。
    /// 这是人类裁定（spec §3 D-3），不是 bug——将来有人"顺手修好"成清零语义，这条会红。
    #[test]
    fn drop_items_does_not_clear_counts_so_dying_after_drops_twice() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
        load_drop_table_1(&mut w, eh);

        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ITEMS, &[]).is_ok());
        assert_eq!(w.body.items.iter_alive().count(), 3, "表 1 展开 = 3 颗");

        // 计数没被清空 —— 随后的死亡效果把同一批再撒一遍。
        w.body.kill_enemy(eh.index as usize, &TABLES_V0);
        assert_eq!(
            w.body.items.iter_alive().count(),
            6,
            "吐完不清空（人类裁定 D-3）：`drop_items(); die();` 掉双份，作者自负"
        );
    }

    /// `die()` 走**完整**死亡效果：四件齐。判别腿——只标 dying 不跑效果的实现会红。
    #[test]
    fn die_runs_the_full_death_effect() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 8888, 100);
        load_drop_table_1(&mut w, eh);
        let score_before = w.body.players[0].score;

        assert!(call(&mut w, &ecl, &mut task, SYS_DIE, &[]).is_ok());

        let i = eh.index as usize;
        assert_eq!(w.body.items.iter_alive().count(), 3, "掉落");
        assert_eq!(
            w.body.players[0].score,
            score_before + 100,
            "加分（测试内不拾取道具，故这 100 只能来自 enemies.score）"
        );
        assert!(
            w.body
                .frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_ENEMY_DIED),
            "死亡事件"
        );
        assert_ne!(
            w.body.enemies.flags[i] & crate::enemy::ENEMY_DYING,
            0,
            "dying 标记"
        );
        let reqs = w.body.take_requests();
        assert!(
            reqs.iter().any(|r| r.id == crate::consts::REQ_ENEMY_DEATH),
            "死亡特效请求"
        );
    }

    /// `die()` 打**满血** boss → hp 归 0（不是留在 8888）。
    /// 防"HUD 当帧显示满血死人"；这也是 `min(0)` 与"什么都不做"的判别点。
    #[test]
    fn die_on_full_hp_enemy_zeroes_hp() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
        let i = eh.index as usize;
        assert_eq!(w.body.enemies.hp_max[i], 8888, "前提：满血");

        assert!(call(&mut w, &ecl, &mut task, SYS_DIE, &[]).is_ok());
        assert_eq!(w.body.enemies.hp[i], 0, "满血 die → hp 压到 0");

        // 反向腿：overkill 打成的负血不得被抹平（负值可观测性是
        // `settle_overkill_two_shots_one_death_event` 的依赖）——证明用的是 `min(0)`
        // 而不是无条件置 0。
        let (eh2, _t2) = enemy_owner_task(&mut w, 10, 0);
        let j = eh2.index as usize;
        w.body.enemies.hp[j] = -5;
        w.body.kill_enemy(j, &TABLES_V0);
        assert_eq!(w.body.enemies.hp[j], -5, "min(0) 而非无条件置 0");
    }

    /// `drop_clear()` 清空计数 → 随后的死亡效果一颗都不掉（`clear_enemy_drops` 的正腿；
    /// 七条钉裁定的测试里只有 misuse 腿碰过 `SYS_DROP_CLEAR`，那条不区分"清空"与"no-op"）。
    #[test]
    fn drop_clear_zeroes_counts_so_death_drops_nothing() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
        load_drop_table_1(&mut w, eh);

        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_CLEAR, &[]).is_ok());
        assert_eq!(
            w.body.enemies.drop_count[eh.index as usize],
            [0; crate::items::ITEM_TYPE_COUNT]
        );
        w.body.kill_enemy(eh.index as usize, &TABLES_V0);
        assert_eq!(w.body.items.iter_alive().count(), 0, "清空后死亡不掉落");
    }

    /// P4-b：`drop_add` 坏类型（≥ `ITEM_TYPE_COUNT` / 负数）→ no-op + contract_viol，无掉落。
    #[test]
    fn drop_add_bad_type_degrades_and_counts() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
        let i = eh.index as usize;
        let viol0 = w.body.diag.contract_viol;

        // 腿一：越界正数类型。取值 `(type=ITEM_TYPE_COUNT=5, n=3)` 是**刻意挑的判别对**：
        // `n=3` 自己是个**合法**类型号（`ITEM_BOMB_PIECE`），所以一旦实现把两个 `pop` 写反
        // （正序参 `type, n` → 逆序弹栈 `n` 先 `type` 后），这条腿就不再是"坏类型 no-op"，
        // 而是老老实实执行 `drop_count[3] += 5` —— 被下面的"全零"断言当场逮住。
        // 换成 `n=1` 之类也合法的值同样能逮，但换成 `n` 越界（如 99）就两路都 no-op，
        // 这条腿会退化成对参数序瞎的测试。别顺手改这两个数。
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_DROP_ADD,
                &[crate::items::ITEM_TYPE_COUNT as i32, 3]
            )
            .is_ok(),
            "坏类型是 P4-b 降级，不 Fault"
        );
        // 腿二：负数类型
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[-1, 3]).is_ok());

        assert_eq!(
            w.body.enemies.drop_count[i],
            [0; crate::items::ITEM_TYPE_COUNT],
            "两条坏类型腿都必须 no-op"
        );
        assert_eq!(
            w.body.diag.contract_viol,
            viol0 + 2,
            "各计一次 contract_viol"
        );
        w.body.kill_enemy(i, &TABLES_V0);
        assert_eq!(w.body.items.iter_alive().count(), 0, "无掉落");
    }

    /// P4-b：`drop_add` 的 n 为负或巨大 → 钳位后饱和，不 panic 不回绕。
    /// `n = -5` → 计数不变（视同 0）；`n = i32::MAX` → 计数封顶 255。
    #[test]
    fn drop_add_clamps_and_saturates_n() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
        let i = eh.index as usize;
        let ty = crate::items::ITEM_POINT as i32;
        let slot = crate::items::ITEM_POINT as usize;

        // 负 n 视同 0（本刀不做"减掉落"；只饱和不钳的实现会让 `-5 as u8` 回绕成 251）。
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, -5]).is_ok());
        assert_eq!(w.body.enemies.drop_count[i][slot], 0, "负 n 视同 0");

        // 增量语义：两次调用累加，不是覆盖。
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, 2]).is_ok());
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, 3]).is_ok());
        assert_eq!(w.body.enemies.drop_count[i][slot], 5, "增量累加");

        // 巨大 n：先钳 [0,255] 再 saturating_add —— 不 panic、不回绕。
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, i32::MAX]).is_ok());
        assert_eq!(w.body.enemies.drop_count[i][slot], 255, "封顶 255");
        // 已封顶再加仍是 255（饱和，非回绕）——只钳不饱和会在这里溢出 panic。
        assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, 200]).is_ok());
        assert_eq!(w.body.enemies.drop_count[i][slot], 255, "饱和不回绕");
    }

    /// spec §5 的断言腿：**`die()` 打在绑卡 boss 上，经现有的破卡三路 OR 自动收卡结算——
    /// 无需新增机制。** 这是设计里一句"现成机制够用"的论断，不是推论，要验（全支线复审
    /// Important #3 补：spec §8 点名的这一行在 T4 交接中蒸发了，全仓再无 `SYS_DIE` 与
    /// 符卡槽同框的测试）。
    ///
    /// 与 `vm::tests::spell_bound_boss_self_destruct_settles_spell_with_hp_above_threshold`
    /// 的分工：那条走 **D9 自燃**（不碰 hp，故 hp 远高于血线，判的是三路 OR 里
    /// `ENEMY_DYING` 那一路的判别力）；本条走 **`SYS_DIE`**，`kill_enemy` 的 `hp.min(0)`
    /// 让第三路 `hp<=threshold` 同真，故它**不**是 OR 分支的判别腿——它验的是别的东西：
    /// syscall → 世界 → `settle_spells` 这条链在 `die()` 上真的接通了。
    #[test]
    fn die_on_spell_bound_boss_settles_the_spell() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 10_000, 0);
        assert!(
            w.body.spell_begin_internal(0, eh, 77, 100, 1000, 0, 300),
            "前提：卡开起来了"
        );
        assert_eq!(w.body.spells[0].active, 1, "前提：槽 active");

        assert!(call(&mut w, &ecl, &mut task, SYS_DIE, &[]).is_ok());
        assert_eq!(
            w.body.enemies.hp[eh.index as usize], 0,
            "前提：`die()` 压过血线下钳（spec §4.2 有意为之），不是停在 threshold=300"
        );

        w.body.settle_spells(&TABLES_V0);
        assert_eq!(
            w.body.spells[0].active, 0,
            "spec §5 断言：`die()` 打绑卡 boss 无需新增机制即收卡结算"
        );
        assert!(
            w.body
                .frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_SPELL_CAPTURED && e.data[0] == 77),
            "走的是正常结算路径（CAPTURED），不是把槽抹了"
        );
    }

    /// misuse：非 enemy-owner 调这四个 → Fault（照 `self_enemy_handle` 既有口径）。
    #[test]
    fn drop_and_die_syscalls_fault_for_non_enemy_owner() {
        let (mut w, ecl) = fresh();
        for (no, args) in [
            (SYS_DROP_CLEAR, &[][..]),
            (SYS_DROP_ADD, &[0, 1][..]),
            (SYS_DROP_ITEMS, &[][..]),
            (SYS_DIE, &[][..]),
        ] {
            let mut task = Task {
                owner_kind: OWNER_STAGE,
                ..Task::default()
            };
            assert_eq!(
                call(&mut w, &ecl, &mut task, no, args),
                Err(FAULT_BAD_OP),
                "syscall {no} 的非敌 owner 腿"
            );
        }
    }

    // ── Shooter 配置面（syscall 600-652；shooter 刀 T2 2026-07-31）──────────────
    //
    // 四条测试全是**表驱动**的：14 个 setter 各写一段复制粘贴既臃肿、又保证将来加第 15 个
    // 时漏掉——而漏掉的那个恰恰是最可能忘写边界检查/写串字段的那个。表在一处，加一行即入网。

    /// 一个 setter 的完整描述：syscall 号 + `id` 之后的实参（正序）+ "它应当造成的改变"。
    /// `mutate` 是**期望的**改变（作用在 `ShooterSlot::default()` 上），与实际派发结果整槽比对
    /// ⇒ 既验"该改的改了"，又验"不该改的一个字节都没动"。
    struct ShCase {
        no: u16,
        /// `id` 之后的实参，脚本声明顺序（正序压栈）。
        rest: &'static [i32],
        mutate: fn(&mut ShooterSlot),
    }

    /// 14 个 setter 的单一权威表（新增 setter 就加一行——四条测试全部自动覆盖它）。
    fn shooter_cases() -> Vec<ShCase> {
        vec![
            // `sh_reset` 作用在默认槽上是恒等——它自己的判别腿在
            // `sh_reset_restores_every_field_to_default`（先弄脏再重置）。
            ShCase {
                no: SYS_SH_RESET,
                rest: &[],
                mutate: |_s| {},
            },
            ShCase {
                no: SYS_SH_SPRITE,
                rest: &[19], // 编译期已折叠成单个 appearance（同 fire/batch）
                mutate: |s| s.appearance = 19,
            },
            ShCase {
                no: SYS_SH_OFFSET,
                rest: &[0x0001_0000, 0x0002_0000],
                mutate: |s| {
                    s.off_x = Fx::from_raw(0x0001_0000);
                    s.off_y = Fx::from_raw(0x0002_0000);
                },
            },
            ShCase {
                no: SYS_SH_OFFSET_ABS,
                rest: &[0x0003_0000, 0x0004_0000],
                mutate: |s| {
                    s.off_x = Fx::from_raw(0x0003_0000);
                    s.off_y = Fx::from_raw(0x0004_0000);
                    s.flags |= SH_ABS_OFFSET;
                },
            },
            ShCase {
                no: SYS_SH_OFFSET_RAD,
                rest: &[0x2000, 0x0005_0000],
                mutate: |s| {
                    s.polar_ang = Angle(0x2000);
                    s.polar_r = Fx::from_raw(0x0005_0000);
                },
            },
            ShCase {
                no: SYS_SH_DIST,
                rest: &[0x0006_0000],
                mutate: |s| s.dist = Fx::from_raw(0x0006_0000),
            },
            ShCase {
                no: SYS_SH_ANGLE,
                rest: &[0x1234, 0x0100],
                mutate: |s| {
                    s.angle0 = Angle(0x1234);
                    s.angle_step = Angle(0x0100);
                },
            },
            ShCase {
                no: SYS_SH_SPEED,
                rest: &[0x0002_8000, -0x0000_4000],
                mutate: |s| {
                    s.speed0 = Fx::from_raw(0x0002_8000);
                    s.speed_step = Fx::from_raw(-0x0000_4000);
                },
            },
            ShCase {
                no: SYS_SH_COUNT,
                rest: &[7, 3],
                mutate: |s| {
                    s.n_angle = 7;
                    s.n_speed = 3;
                },
            },
            ShCase {
                no: SYS_SH_AIM,
                rest: &[1],
                mutate: |s| s.flags |= SH_AIMED,
            },
            ShCase {
                no: SYS_SH_RING,
                rest: &[1],
                mutate: |s| s.flags |= SH_RING,
            },
            ShCase {
                no: SYS_SH_XFORM,
                rest: &[9, 2], // codegen 把 XformRef 降低成 (off, cnt) 两个栈值
                mutate: |s| {
                    s.xform_off = 9;
                    s.xform_cnt = 2;
                },
            },
            ShCase {
                no: SYS_SH_TASK,
                rest: &[3],
                mutate: |s| s.task_script = 3,
            },
            ShCase {
                no: SYS_SH_REQ,
                rest: &[42],
                mutate: |s| s.on_fire_req = 42,
            },
        ]
    }

    /// `id` 打头的完整实参（正序）。
    fn sh_args(id: i32, rest: &[i32]) -> Vec<i32> {
        let mut v = vec![id];
        v.extend_from_slice(rest);
        v
    }

    /// `fresh()` + 把 0 号任务的 shooter 槽置成 `spawn` 会给的初值。
    ///
    /// ⚠️ **必须显式做这一步**：`World::new` 走 `alloc_zeroed`（见 `step::World::new`），
    /// 而 `ShooterSlot::default()` **非全零**（`n_angle/n_speed=1`、`task_script=0xFFFF`）——
    /// 生产路径上是 `TaskPool::spawn` 负责重置，而本模块的 `call` 助手用的是手拼 `Task`
    /// （从没经过 `spawn`）。不补这一步，测试比对的基线就是"全零槽"而非真实初值，
    /// `sh_reset` 一类断言会以看不出所以然的方式红。坑档见 `TaskPool::new` 文档。
    fn fresh_with_shooters() -> (Box<World>, EclImage) {
        let (mut w, ecl) = fresh();
        w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        (w, ecl)
    }

    /// 每个 setter 只改自己那一维，其余字段纹丝不动。
    /// 这条守的是"14 个派发臂没有互相串写"——串写在只测单个 setter 的测试里看不出来。
    #[test]
    fn each_shooter_setter_touches_only_its_own_field() {
        for c in shooter_cases() {
            let (mut w, ecl) = fresh_with_shooters();
            let mut task = Task::default();
            assert!(
                call(&mut w, &ecl, &mut task, c.no, &sh_args(1, c.rest)).is_ok(),
                "syscall {} 应正常返回",
                c.no
            );
            let mut want = ShooterSlot::default();
            (c.mutate)(&mut want);
            assert_eq!(
                w.tasks.shooters[0][1], want,
                "syscall {} 改动的字段集不符（多改/少改/串写）",
                c.no
            );
            for k in [0usize, 2, 3] {
                assert_eq!(
                    w.tasks.shooters[0][k],
                    ShooterSlot::default(),
                    "syscall {} 串写到了槽 {k}",
                    c.no
                );
            }
            assert_eq!(task.sp, 0, "syscall {} 未把实参全部弹栈", c.no);
        }
    }

    /// `sh_offset` 与 `sh_offset_abs` 写的是同一对字段，只是解释方式不同——**后写的赢**。
    /// 判别腿：先 abs 后 rel，`SH_ABS_OFFSET` 必须被**清掉**（不是只置不清）。
    #[test]
    fn offset_abs_flag_is_set_and_cleared_by_the_two_setters() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        let ten = Fx::from_int(10).raw();
        let twenty = Fx::from_int(20).raw();
        let thirty = Fx::from_int(30).raw();
        let forty = Fx::from_int(40).raw();

        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SH_OFFSET_ABS,
                &[0, ten, twenty]
            )
            .is_ok()
        );
        let s = w.tasks.shooters[0][0];
        assert_ne!(s.flags & SH_ABS_OFFSET, 0, "abs 设完标志应置位");
        assert_eq!((s.off_x.raw(), s.off_y.raw()), (ten, twenty));

        assert!(call(&mut w, &ecl, &mut task, SYS_SH_OFFSET, &[0, thirty, forty]).is_ok());
        let s = w.tasks.shooters[0][0];
        assert_eq!(
            s.flags & SH_ABS_OFFSET,
            0,
            "相对模式必须把 SH_ABS_OFFSET **清掉**（只置不清 = 一旦 abs 过就再也回不去）"
        );
        assert_eq!((s.off_x.raw(), s.off_y.raw()), (thirty, forty), "后写的赢");
    }

    /// `sh_reset` 恢复**全部**默认（不只是清几个字段）。
    #[test]
    fn sh_reset_restores_every_field_to_default() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        // 14 个 setter 全调一遍把槽 0 弄脏（`sh_reset` 自己那行是恒等，无所谓）。
        for c in shooter_cases() {
            assert!(call(&mut w, &ecl, &mut task, c.no, &sh_args(0, c.rest)).is_ok());
        }
        assert_ne!(
            w.tasks.shooters[0][0],
            ShooterSlot::default(),
            "前置条件：整套 setter 跑完槽必须确实脏了（否则本测试是假绿）"
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_RESET, &[0]).is_ok());
        assert_eq!(
            w.tasks.shooters[0][0],
            ShooterSlot::default(),
            "sh_reset 必须恢复整槽默认，不是清几个字段"
        );
    }

    /// P4-b：`id ≥ SHOOTERS_PER_TASK` → no-op + contract_viol + STATUS_BAD_ARGS。
    /// **14 个 setter 各一腿**（表驱动）——只测其中一个的话，漏写边界检查的那几个不会红。
    #[test]
    fn every_shooter_setter_rejects_out_of_range_id() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        let mut expect_viol = w.body.diag.contract_viol;
        for c in shooter_cases() {
            for bad_id in [SHOOTERS_PER_TASK as i32, -1] {
                w.body.last_status = crate::world::STATUS_OK;
                assert!(
                    call(&mut w, &ecl, &mut task, c.no, &sh_args(bad_id, c.rest)).is_ok(),
                    "syscall {} 越界 id 是 no-op 而非 Fault",
                    c.no
                );
                expect_viol += 1;
                assert_eq!(
                    w.body.diag.contract_viol, expect_viol,
                    "syscall {} 的 id={bad_id} 腿应记一次 contract_viol",
                    c.no
                );
                assert_eq!(
                    w.body.last_status,
                    crate::world::STATUS_BAD_ARGS,
                    "syscall {} 的 id={bad_id} 腿应置 STATUS_BAD_ARGS",
                    c.no
                );
                // no-op 的判别腿：只查 contract_viol 证不了"没写脏"。
                for k in 0..SHOOTERS_PER_TASK {
                    assert_eq!(
                        w.tasks.shooters[0][k],
                        ShooterSlot::default(),
                        "syscall {} 的 id={bad_id} 腿写脏了槽 {k}",
                        c.no
                    );
                }
                assert_eq!(task.sp, 0, "syscall {} 越界腿也应把实参全部弹栈", c.no);
            }
        }
    }

    /// 复审 ②：`sh_aim`/`sh_ring` 的**清位**方向。
    ///
    /// `shooter_cases()` 那张表每个 setter 只喂一组入参，两条旗标 setter 喂的都是 `on = 1`
    /// ⇒ 把两个派发臂里的 `else { s.flags &= !BIT }` **整个删掉，表驱动那三条测试全绿**。
    /// 这跟 `sh_offset` 的"只置不清"是同一个 bug 类，只是表结构塞不下"同一 setter 两种
    /// 入参"，故另开一条定向腿。
    #[test]
    fn aim_and_ring_flags_are_cleared_by_passing_zero() {
        for (no, bit, who) in [
            (SYS_SH_AIM, SH_AIMED, "sh_aim"),
            (SYS_SH_RING, SH_RING, "sh_ring"),
        ] {
            let (mut w, ecl) = fresh_with_shooters();
            let mut task = Task::default();
            assert!(call(&mut w, &ecl, &mut task, no, &[0, 1]).is_ok());
            assert_ne!(w.tasks.shooters[0][0].flags & bit, 0, "{who}(id, 1) 应置位");
            assert!(call(&mut w, &ecl, &mut task, no, &[0, 0]).is_ok());
            assert_eq!(
                w.tasks.shooters[0][0].flags & bit,
                0,
                "{who}(id, 0) 必须把位**清掉**（只置不清 = 开了就再也关不上）"
            );
            // 判别腿：清的是自己那一位，没顺手把另一位也抹了。
            assert_eq!(
                w.tasks.shooters[0][0],
                ShooterSlot::default(),
                "{who} 一置一清之后整槽应回到默认"
            );
        }
    }

    /// 复审 ③：shooter 存储按**任务索引**键——派发臂必须用 `ctx.self_index`，不是字面量 0。
    /// 助手把 `self_index` 写死 0 时这条是等价变异（改成 `0` 全绿）。
    #[test]
    fn setters_write_the_slots_of_the_calling_task_not_task_zero() {
        let (mut w, ecl) = fresh();
        w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        w.tasks.shooters[7] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        let mut task = Task::default();
        assert!(call_at(&mut w, &ecl, &mut task, SYS_SH_DIST, &[2, 0x0006_0000], 7).is_ok());
        assert_eq!(
            w.tasks.shooters[7][2].dist,
            Fx::from_raw(0x0006_0000),
            "写的应是 self_index=7 的槽 2"
        );
        for k in 0..SHOOTERS_PER_TASK {
            assert_eq!(
                w.tasks.shooters[0][k],
                ShooterSlot::default(),
                "0 号任务的槽 {k} 不该被碰（派发臂若写死 0 就会红在这里）"
            );
        }
    }

    /// 复审 ①：`sh_sprite` 的收窄**必须保号越界性**。
    ///
    /// 负折叠值（`sh_sprite(0, 16, c)` 里 `c` 是变量、typeck 判据跳过 ⇒ `c = -20` 折成 -4）
    /// 若被钳成 **0**，就落在 `appearances[0]` 这个**在册且 valid** 的格子上 ⇒ T3 开火时
    /// 再也拒不掉。`fire` 侧同样的值是 `FAULT_BAD_OP`，两条路不该有这种差别。
    #[test]
    fn sh_sprite_narrowing_keeps_out_of_range_values_out_of_range() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        let n_rows = TABLES_V0.appearances.len();
        assert!(
            TABLES_V0.appearances[0].valid,
            "前置条件：0 号格在册且 valid——正因为如此，把负值钳成 0 才是事故"
        );

        for bad in [-4, -1, i32::MIN] {
            assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, bad]).is_ok());
            let got = w.tasks.shooters[0][0].appearance;
            assert_ne!(got, 0, "负 appearance {bad} 不得被洗成合法的 0 号格");
            assert!(
                got as usize >= n_rows,
                "负 appearance {bad} 收窄后仍须落在表外（表 {n_rows} 行，实际存了 {got}）"
            );
        }
        // 上沿：超 u16 同样保持越界。
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, 100_000]).is_ok());
        assert!(w.tasks.shooters[0][0].appearance as usize >= n_rows);
        // 合法值照常原样存（防"一律存 u16::MAX"这种把测试骗绿的实现）。
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, 19]).is_ok());
        assert_eq!(w.tasks.shooters[0][0].appearance, 19);
    }

    /// `sh_req` 的收窄（复审 ① 顺带点名的覆盖空缺）。这里钳到 **0 是对的**——`0` 在
    /// `ShooterSlot::on_fire_req` 上有明确语义（"不发请求"），不像 appearance 的 0 是个
    /// 在册格子；负 req id 降级成"不发"是 P4-b 的确定性安全结果。
    #[test]
    fn sh_req_narrowing_degrades_bad_ids_to_no_request() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_REQ, &[0, -7]).is_ok());
        assert_eq!(w.tasks.shooters[0][0].on_fire_req, 0, "负 id → 不发请求");
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_REQ, &[0, 100_000]).is_ok());
        assert_eq!(
            w.tasks.shooters[0][0].on_fire_req,
            u16::MAX,
            "超 u16 上钳，不回绕（裸 `as u16` 会得 34464）"
        );
    }

    /// P4-b：`sh_count` 的参数是脚本给的任意 i32，先钳后存，不回绕不 panic。
    #[test]
    fn sh_count_clamps_both_ends() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, -5, i32::MAX]).is_ok());
        let s = w.tasks.shooters[0][0];
        assert_eq!(s.n_angle, 0, "负数下钳到 0（裸 `as u8` 会回绕成 251）");
        assert_eq!(s.n_speed, 255, "i32::MAX 上钳到 255");
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, 256, -256]).is_ok());
        let s = w.tasks.shooters[0][0];
        assert_eq!(s.n_angle, 255, "256 上钳到 255（裸 `as u8` 会回绕成 0）");
        assert_eq!(s.n_speed, 0, "-256 下钳到 0");
    }

    // ── Shooter 开火面（syscall 660；shooter 刀 T3 2026-07-31）────────────────────
    //
    // 开火循环住 **ECL 层**（spec §10 末：P1 使然——world 不知道任务存在、没法逐颗挂
    // `task_script`），于是网格逻辑有了**第二份实现**。故下面第一条等价测试是**必需**
    // 而非顺带的：它一次押住网格序、坐标算法、速度递增、以及居中公式本身。

    /// 配置/开火用的 `call` 包装：顺带把"实参必须全部弹栈"这条查到每一次调用上
    /// （越界腿也算——shooter 族的栈效应与成功路径一致，见 `shooter_mut` 文档）。
    fn sh(w: &mut World, ecl: &EclImage, task: &mut Task, no: u16, args: &[i32]) {
        assert!(
            call(w, ecl, task, no, args).is_ok(),
            "shooter syscall {no} 不应 Fault"
        );
        assert_eq!(task.sp, 0, "shooter syscall {no} 未把实参全部弹栈");
    }

    /// 灌池用的哑弹模板。
    fn filler_bullet() -> BulletInit {
        BulletInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: Angle::ZERO,
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
            transform_head: crate::xform::XFORM_NONE,
            xform_wait: 0,
            xform_next: 0,
            born_frame: 0,
        }
    }

    /// shooter 的 fan 与手写 `batch` 等价——**带居中补偿**。
    /// 一次押住网格序、坐标算法、速度递增、**以及居中公式本身**。
    ///
    /// 为什么必需：开火循环住 ECL 层（P1 使然，见 spec §10 末），网格逻辑有了第二份实现。
    /// 补偿写错、或实现忘了居中，这条都会红。
    #[test]
    fn shooter_fan_matches_batch_with_centering_compensation() {
        const N_ANGLE: i32 = 5;
        const N_SPEED: i32 = 3;
        const BASE: i32 = 0x1234;
        const STEP: i32 = 0x0400;
        let x = Fx::from_int(30).raw();
        let y = Fx::from_int(-40).raw();
        let speed0 = Fx::from_int(2).raw();
        let speed_step = 0x0000_8000; // 0.5

        // 世界 A：shooter。owner 是 STAGE ⇒ `self_pos` 恒 (0,0)，故 `sh_offset` 就是原点。
        // 其余全默认：无 aim / ring / dist / polar / xform / 挂弹任务。
        let (mut wa, ecl) = fresh_with_shooters();
        let mut ta = Task::default();
        sh(&mut wa, &ecl, &mut ta, SYS_SH_SPRITE, &[0, ROW_C]);
        sh(&mut wa, &ecl, &mut ta, SYS_SH_OFFSET, &[0, x, y]);
        sh(&mut wa, &ecl, &mut ta, SYS_SH_COUNT, &[0, N_ANGLE, N_SPEED]);
        sh(&mut wa, &ecl, &mut ta, SYS_SH_ANGLE, &[0, BASE, STEP]);
        sh(
            &mut wa,
            &ecl,
            &mut ta,
            SYS_SH_SPEED,
            &[0, speed0, speed_step],
        );
        sh(&mut wa, &ecl, &mut ta, SYS_SH_FIRE, &[0]);

        // 世界 B：手写 `batch`，基准角**自带居中补偿**。
        let compensated = BASE - (N_ANGLE - 1) * STEP / 2;
        assert_ne!(
            compensated, BASE,
            "前置条件：这组参数下补偿必须非恒等，否则'两边都忘了居中'也能绿"
        );
        let (mut wb, _) = fresh();
        let mut tb = Task::default();
        let args = [
            ROW_C,
            x,
            y,
            N_ANGLE,
            compensated,
            STEP,
            N_SPEED,
            speed0,
            speed_step,
        ];
        assert!(call(&mut wb, &ecl, &mut tb, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
        assert_eq!(tb.stack[0], N_ANGLE * N_SPEED, "batch 侧应发满 5×3");

        let ia: Vec<usize> = wa.body.bullets.iter_alive().collect();
        let ib: Vec<usize> = wb.body.bullets.iter_alive().collect();
        assert_eq!(
            ia.len(),
            (N_ANGLE * N_SPEED) as usize,
            "shooter 侧应发满 5×3"
        );
        assert_eq!(ia.len(), ib.len());
        for (k, (&a, &b)) in ia.iter().zip(ib.iter()).enumerate() {
            assert_eq!(wa.body.bullets.x[a], wb.body.bullets.x[b], "第 {k} 颗 x");
            assert_eq!(wa.body.bullets.y[a], wb.body.bullets.y[b], "第 {k} 颗 y");
            assert_eq!(wa.body.bullets.vx[a], wb.body.bullets.vx[b], "第 {k} 颗 vx");
            assert_eq!(wa.body.bullets.vy[a], wb.body.bullets.vy[b], "第 {k} 颗 vy");
            assert_eq!(
                wa.body.bullets.speed[a], wb.body.bullets.speed[b],
                "第 {k} 颗 speed"
            );
            assert_eq!(
                wa.body.bullets.angle[a], wb.body.bullets.angle[b],
                "第 {k} 颗 angle"
            );
            assert_eq!(
                wa.body.bullets.sprite[a], wb.body.bullets.sprite[b],
                "第 {k} 颗 sprite"
            );
            assert_eq!(
                wa.body.bullets.radius[a], wb.body.bullets.radius[b],
                "第 {k} 颗 radius"
            );
        }
        // 判别腿的判别腿：网格里必须真的出现过多个不同角、多个不同速，否则上面逐一比
        // 是在比一堆相同的弹（`n_angle`/`n_speed` 被实现忽略掉也能绿）。
        let mut angles: Vec<u16> = ia.iter().map(|&i| wa.body.bullets.angle[i].raw()).collect();
        let mut speeds: Vec<i32> = ia.iter().map(|&i| wa.body.bullets.speed[i].raw()).collect();
        angles.sort_unstable();
        angles.dedup();
        speeds.sort_unstable();
        speeds.dedup();
        assert_eq!(angles.len(), N_ANGLE as usize, "网格里应出现 5 个不同角");
        assert_eq!(speeds.len(), N_SPEED as usize, "网格里应出现 3 个不同速");
    }

    /// 居中公式的**偶数路 × 负步长**格（复审 ①/③）。
    ///
    /// 上一版这两格全无覆盖：等价测试只跑 `N_ANGLE = 5`（奇数），唯一用偶数路的
    /// `dist_pushes_each_bullet_along_its_own_angle` 只比 dist 前后的**位移差**、两侧
    /// `fan_center` 相同 ⇒ 对居中值完全免疫。于是下面这个 bug 整整逃逸了一轮：
    ///
    /// `ShooterSlot.angle_step` 存的是 `Angle`（底层 **u16**），`sh_angle(0, 0deg, -6deg)`
    /// 这种常规写法存进去的是 `65536 − 1092`。`i·step`/`j·step` 在 mod 65536 下不受影响，
    /// **但 `/2` 不与 mod 65536 交换**：
    ///   `((n−1)(s + 65536))/2 = ((n−1)s)/2 + (n−1)·32768`
    /// `n` 奇数 ⇒ `(n−1)` 偶 ⇒ 多出项是 65536 的整数倍、无害；
    /// **`n` 偶数 ⇒ 多出 32768 BAM = 半圈**，整把扇形被搬到 `base` 的正对面
    /// （扇形形状还对，相邻仍差 `step`——所以"形状对不对"式的测试也看不见它）。
    ///
    /// 修法是读侧符号扩展（`as i16 as i32`），回到本仓家规：`create_bullets_batch` 的形参
    /// 就是 `angle_step: i16`、`Angle::add_delta` 也收 i16——**有符号角度增量是既定口径**，
    /// `ShooterSlot` 存 `Angle` 是唯一破口。别改结构（槽宽已被 44 B 测试冻结）。
    #[test]
    fn fan_centering_handles_even_ways_with_negative_step() {
        const N_ANGLE: i32 = 4;
        const STEP: i32 = -0x400; // 脚本侧的 `-Xdeg`：经 `bam()` 存成 0xFC00
        const BASE: i32 = 0;

        let (mut wa, ecl) = fresh_with_shooters();
        let mut ta = Task::default();
        sh(&mut wa, &ecl, &mut ta, SYS_SH_SPRITE, &[0, ROW_C]);
        sh(&mut wa, &ecl, &mut ta, SYS_SH_COUNT, &[0, N_ANGLE, 1]);
        sh(&mut wa, &ecl, &mut ta, SYS_SH_ANGLE, &[0, BASE, STEP]);
        sh(&mut wa, &ecl, &mut ta, SYS_SH_FIRE, &[0]);
        let got: Vec<u16> = wa
            .body
            .bullets
            .iter_alive()
            .map(|i| wa.body.bullets.angle[i].raw())
            .collect();

        // 契约意图：base ± 512、base ± 1536（4 路以 base 为中心、步长 −1024）。
        // 有 bug 的实现给出 34304/33280/32256/31232——形状对、整扇偏 180°。
        assert_eq!(
            got,
            vec![1536, 512, 65024, 64000],
            "偶数路 + 负步长的居中值错了（差 32768 = 半圈就是 u16 零扩展那个 bug）"
        );

        // 同一格也走一遍与 `batch` 的等价（`batch` 的 `angle_step` 形参本就是 i16，
        // 故它是这条口径的现成参照物）。
        let compensated = BASE - (N_ANGLE - 1) * STEP / 2;
        let (mut wb, _) = fresh();
        let mut tb = Task::default();
        let args = [ROW_C, 0, 0, N_ANGLE, compensated, STEP, 1, 0, 0];
        assert!(call(&mut wb, &ecl, &mut tb, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
        let want: Vec<u16> = wb
            .body
            .bullets
            .iter_alive()
            .map(|i| wb.body.bullets.angle[i].raw())
            .collect();
        assert_eq!(
            got, want,
            "偶数路 + 负步长下 shooter 的 fan 应仍与 batch 等价"
        );
    }

    /// 复审 ②：aim 的基点是**原点**（含全部三种偏移），不是 owner 位置。
    ///
    /// 为什么另开一条：`aim_resolves_at_fire_time_not_at_set_time` 的 owner 是 STAGE、
    /// 无任何偏移 ⇒ `origin == self_pos == (0,0)`，两个基点**重合**——把实现里的
    /// `origin_x/origin_y` 换成 `self_pos(task, ctx)`（也就是"顺手复用
    /// `sys_aim_player_angle` 口径"这个最自然的错法）那条测试照样绿。
    /// 同 `CLAUDE.md` 点名的"圆心重合式测试对半径映射是瞎的"。
    #[test]
    fn aim_is_measured_from_the_fire_origin_not_from_the_owner() {
        let (mut w, ecl) = fresh_with_shooters();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 100, -60, 5);
        let mut t = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        // 绝对偏移把原点挪到 (300, 0)——**远离** owner 的 (100, -60)。
        sh(
            &mut w,
            &ecl,
            &mut t,
            SYS_SH_OFFSET_ABS,
            &[0, Fx::from_int(300).raw(), 0],
        );
        sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

        let i = w.body.bullets.iter_alive().next().expect("应发一颗");
        let got = w.body.bullets.angle[i];
        let (px, py) = (w.body.players[0].x, w.body.players[0].y);
        let from_origin = crate::math::cordic::atan2(py - Fx::ZERO, px - Fx::from_int(300));
        let from_owner = crate::math::cordic::atan2(py - Fx::from_int(-60), px - Fx::from_int(100));
        // 两条断言都要：只写前一条的话，owner 位置恰好也对得上的取值会漏。
        assert_eq!(got, from_origin, "aim 必须从**开火原点**量");
        assert_ne!(
            got, from_owner,
            "aim 从 owner 位置量了（顺手复用 sys_aim_player_angle 的口径就是这个结果）"
        );
    }

    /// 判别式（F8）：`sh_aim` 瞄的也是**最近的可瞄自机**，与 `aim_player()` 同一条规矩。
    /// owner 是 STAGE、无偏移 ⇒ 出弹口在原点；P0 尸体在左上、P1 活着在右下。
    #[test]
    fn shooter_aim_skips_unaimable_player() {
        let (mut w, ecl) = fresh_with_shooters();
        w.body.players[0].x = Fx::from_int(-200);
        w.body.players[0].y = Fx::from_int(-150);
        w.body.players[0].life_state = crate::player::LIFE_GAMEOVER;
        w.body.players[1] = crate::player::PlayerState::spawn(0, &TABLES_V0.characters[0]);
        w.body.players[1].x = Fx::from_int(200);
        w.body.players[1].y = Fx::from_int(150);
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

        let i = w.body.bullets.iter_alive().next().expect("应发一颗");
        let to_p1 = crate::math::cordic::atan2(Fx::from_int(150), Fx::from_int(200));
        let to_p0 = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
        assert_eq!(w.body.bullets.angle[i], to_p1, "发射器应瞄唯一可瞄的 P1");
        assert_ne!(
            w.body.bullets.angle[i], to_p0,
            "发射器瞄了已 GAMEOVER 的 P0 尸体坐标"
        );
    }

    /// 同上的 fallback 腿：发射器这条路也**必须产出角度**，无可瞄自机时回退到 P0 最后坐标
    /// （与 `sys_aim_player_angle_falls_back_to_p0_when_none_aimable` 成对，两条路同一口径）。
    #[test]
    fn shooter_aim_falls_back_to_p0_when_none_aimable() {
        let (mut w, ecl) = fresh_with_shooters();
        w.body.players[0].x = Fx::from_int(-200);
        w.body.players[0].y = Fx::from_int(-150);
        w.body.players[0].life_state = crate::player::LIFE_GAMEOVER; // players[1] 本就 ABSENT
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

        let i = w.body.bullets.iter_alive().next().expect("应发一颗");
        let to_corpse = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
        assert_eq!(
            w.body.bullets.angle[i], to_corpse,
            "无可瞄自机时发射器应回退到 P0 的最后坐标"
        );
    }

    /// `dist` 是**逐颗沿各自角度**位移，不是整环朝同一方向平移。
    /// 判别腿：取一个 n_angle=4、angle_step=90deg 的十字环，四颗弹的位移方向必须两两不同；
    /// 错误实现（整体平移）会让四颗的 (x,y) 相对无 dist 时的偏移量**完全相同**。
    #[test]
    fn dist_pushes_each_bullet_along_its_own_angle() {
        let fire = |dist: i32| -> Vec<(i32, i32)> {
            let (mut w, ecl) = fresh_with_shooters();
            let mut t = Task::default();
            sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
            sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 4, 1]);
            sh(&mut w, &ecl, &mut t, SYS_SH_ANGLE, &[0, 0, 16384]); // 十字：90° 一颗
            sh(&mut w, &ecl, &mut t, SYS_SH_DIST, &[0, dist]);
            sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
            w.body
                .bullets
                .iter_alive()
                .map(|i| (w.body.bullets.x[i].raw(), w.body.bullets.y[i].raw()))
                .collect()
        };
        let base = fire(0);
        let pushed = fire(Fx::from_int(20).raw());
        assert_eq!(base.len(), 4);
        assert_eq!(pushed.len(), 4);
        let off: Vec<(i32, i32)> = base
            .iter()
            .zip(&pushed)
            .map(|(b, p)| (p.0 - b.0, p.1 - b.1))
            .collect();
        for (k, o) in off.iter().enumerate() {
            assert_ne!(*o, (0, 0), "第 {k} 颗根本没被 dist 推开");
        }
        for i in 0..4 {
            for j in (i + 1)..4 {
                assert_ne!(
                    off[i], off[j],
                    "第 {i}/{j} 颗的 dist 位移量相同 ⇒ 实现把整环朝同一方向平移了"
                );
            }
        }
    }

    /// `sh_offset` 与 `sh_offset_rad` 同时设 → 两者**相加**（ZUN 626 明写 stacks）。
    /// 判别腿：只设其一 / 只设另一 / 两个都设，第三种的原点必须等于前两种偏移量之和。
    #[test]
    fn rect_and_polar_offsets_stack_rather_than_override() {
        let x = Fx::from_int(30).raw();
        let y = Fx::from_int(-10).raw();
        let ang = 8192; // 45°
        let r = Fx::from_int(25).raw();
        // owner 是 STAGE ⇒ 基准原点 (0,0)，故"原点"就是偏移量本身。
        let fire = |rect: bool, polar: bool| -> (i32, i32) {
            let (mut w, ecl) = fresh_with_shooters();
            let mut t = Task::default();
            sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
            if rect {
                sh(&mut w, &ecl, &mut t, SYS_SH_OFFSET, &[0, x, y]);
            }
            if polar {
                sh(&mut w, &ecl, &mut t, SYS_SH_OFFSET_RAD, &[0, ang, r]);
            }
            sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
            let i = w
                .body
                .bullets
                .iter_alive()
                .next()
                .expect("默认 1×1 应恰发一颗");
            (w.body.bullets.x[i].raw(), w.body.bullets.y[i].raw())
        };
        let only_rect = fire(true, false);
        let only_polar = fire(false, true);
        let both = fire(true, true);
        assert_ne!(
            only_rect, only_polar,
            "前置条件：两种偏移必须给出不同原点，否则'覆盖'与'相加'不可辨"
        );
        assert_eq!(
            both,
            (only_rect.0 + only_polar.0, only_rect.1 + only_polar.1),
            "两种偏移必须**相加**（后设覆盖先设就会红）"
        );
    }

    /// `aim` 在**开火那一刻**解析，不是 `sh_aim` 时。
    /// 判别腿：sh_aim(0,1) → 移动自机 → sh_fire，基准角必须跟着新位置变。
    /// 错误实现（设的时候就把角算死）会让两次开火的角相同。
    #[test]
    fn aim_resolves_at_fire_time_not_at_set_time() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);

        // 第一发：自机在出场点。原点是 (0,0)（STAGE owner、无偏移），故基准角 = atan2(自机)。
        let p0 = (w.body.players[0].x, w.body.players[0].y);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        // 挪自机——shooter 的参数一个字节都没改。
        w.body.players[0].x = Fx::from_int(200);
        w.body.players[0].y = Fx::from_int(-150);
        let p1 = (w.body.players[0].x, w.body.players[0].y);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

        let idx: Vec<usize> = w.body.bullets.iter_alive().collect();
        assert_eq!(idx.len(), 2);
        let a0 = w.body.bullets.angle[idx[0]];
        let a1 = w.body.bullets.angle[idx[1]];
        assert_eq!(
            a0,
            crate::math::cordic::atan2(p0.1, p0.0),
            "第一发朝旧自机位"
        );
        assert_eq!(
            a1,
            crate::math::cordic::atan2(p1.1, p1.0),
            "第二发朝新自机位——aim 必须在开火那一刻解析"
        );
        assert_ne!(a0, a1, "两发同角 ⇒ 实现在 sh_aim 那一刻就把角算死了");
    }

    /// ring 精确闭合：n_angle=28 时 28 颗**铺满整 65536**、收尾无缝。
    /// 防照抄 demo 的 `65536/n` 预乘写法（2340×28 = 65520，收尾留 16 BAM 的缝）。
    #[test]
    fn ring_distributes_exactly_around_the_full_circle() {
        const N: usize = 28;
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_RING, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, N as i32, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        let a: Vec<u16> = w
            .body
            .bullets
            .iter_alive()
            .map(|i| w.body.bullets.angle[i].raw())
            .collect();
        assert_eq!(a.len(), N);

        // 主判据一：逐颗算 (i×65536)/N，把余数均摊——**不是** i×(65536/N)。
        for (i, &got) in a.iter().enumerate() {
            let want = ((i as i32 * 65536) / N as i32) as u16;
            assert_eq!(got, want, "第 {i} 颗角不是均摊余数的 (i×65536)/N");
        }
        // 主判据二：把**闭合那一段**也算进来，全部相邻角差的极差 ≤ 1。
        // 预乘写法前 27 段都是 2340、闭合段却是 2356 ⇒ 极差 16，红在这里。
        let gaps: Vec<u32> = (0..N)
            .map(|i| a[(i + 1) % N].wrapping_sub(a[i]) as u32)
            .collect();
        let lo = *gaps.iter().min().unwrap();
        let hi = *gaps.iter().max().unwrap();
        assert!(hi - lo <= 1, "相邻角差极差 {} ⇒ 收尾留了缝", hi - lo);
        // 顺带：一整圈。**这条单独是弱判据**——任何绕行一圈的排布都满足它（预乘写法
        // 也满足：27×2340 + 2356 = 65536），故主力是上面两条。
        assert_eq!(gaps.iter().sum::<u32>(), 65536);
    }

    /// ring 模式下 `angle_step` 是**逐层**偏移（而非逐弹）——两层的同序号弹角差 == angle_step。
    #[test]
    fn ring_angle_step_offsets_layers_not_bullets() {
        const STEP: i32 = 0x0300;
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_RING, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 4, 2]);
        sh(&mut w, &ecl, &mut t, SYS_SH_ANGLE, &[0, 0, STEP]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        let a: Vec<u16> = w
            .body
            .bullets
            .iter_alive()
            .map(|i| w.body.bullets.angle[i].raw())
            .collect();
        assert_eq!(a.len(), 8, "序 = 角度外层、速度内层 ⇒ 索引 2i+j");
        for i in 0..4 {
            assert_eq!(
                a[i * 2 + 1].wrapping_sub(a[i * 2]),
                STEP as u16,
                "第 {i} 组两层的角差应恰是 angle_step（逐层，不是逐弹）"
            );
        }
        assert_eq!(
            a[2].wrapping_sub(a[0]),
            (65536 / 4) as u16,
            "同层相邻两颗的角差是均分整周，不是 angle_step"
        );
    }

    /// xform 延迟读安全：`sh_xform` 之后 **wait 跨帧** 再 `sh_fire`，xform 仍生效。
    ///
    /// xform 数据住**本任务的 `locals`**（`task.locals[xform_off..]`），由编译器在 sub 入口
    /// 一次性 staging，`slots` 的调用图着色保证区间不被复用——所以存 `(off, cnt)`、开火时
    /// 才读是安全的。这条用真字节码跑完整 `run_tasks` 循环把它钉死。
    #[test]
    fn xform_survives_a_frame_boundary_between_set_and_fire() {
        // 槽 0：wait=3, op=OP_SET_SPEED(10), args=[Fx::from_int(2).raw(), 0]
        let word0 = ((3u32 << 16) | ((crate::xform::OP_SET_SPEED as u32) << 8)) as i32;
        let arg0 = Fx::from_int(2).raw();
        let code = vec![
            crate::ecl::ops::OP_END as u32,   //  0  sub0 Root（占位）
            crate::ecl::ops::OP_PUSHI as u32, //  1  sub1 Async 入口：locals[0..3] = xform 槽
            word0 as u32,                     //  2
            crate::ecl::ops::OP_POPL as u32,  //  3
            0,                                //  4
            crate::ecl::ops::OP_PUSHI as u32, //  5
            arg0 as u32,                      //  6
            crate::ecl::ops::OP_POPL as u32,  //  7
            1,                                //  8
            crate::ecl::ops::OP_PUSHI as u32, //  9
            0,                                // 10
            crate::ecl::ops::OP_POPL as u32,  // 11
            2,                                // 12
            crate::ecl::ops::OP_PUSHI as u32, // 13  sh_sprite(0, ROW_A)
            0,                                // 14
            crate::ecl::ops::OP_PUSHI as u32, // 15
            ROW_A as u32,                     // 16
            crate::ecl::ops::OP_SYS as u32,   // 17
            SYS_SH_SPRITE as u32,             // 18
            crate::ecl::ops::OP_PUSHI as u32, // 19  sh_xform(0, off=0, cnt=1)
            0,                                // 20
            crate::ecl::ops::OP_PUSHI as u32, // 21
            0,                                // 22
            crate::ecl::ops::OP_PUSHI as u32, // 23
            1,                                // 24
            crate::ecl::ops::OP_SYS as u32,   // 25
            SYS_SH_XFORM as u32,              // 26
            crate::ecl::ops::OP_PUSHI as u32, // 27  wait(2) ← **跨帧**
            2,                                // 28  （2026-08-01 语义修正：旧 `wait(1)` 就是
            //                                        "隔一帧跑"，新语义下等价写法是 wait(2)。
            //                                        这里要的是"设 xform 与开火之间夹一个
            //                                        任务不执行的空转帧"，故跟着改数不改结构。）
            crate::ecl::ops::OP_WAIT as u32,  // 29
            crate::ecl::ops::OP_PUSHI as u32, // 30  sh_fire(0)
            0,                                // 31
            crate::ecl::ops::OP_SYS as u32,   // 32
            SYS_SH_FIRE as u32,               // 33
            crate::ecl::ops::OP_PUSHI as u32, // 34  wait(999)：停在这里别自然结束
            999,                              // 35
            crate::ecl::ops::OP_WAIT as u32,  // 36
            crate::ecl::ops::OP_END as u32,   // 37
        ];
        let ecl = test_image(
            code,
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(1, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("xf_task", 1)],
            Some(0),
        );
        let mut w = World::new(1);
        let idx = w
            .spawn_sub_internal(&ecl, ecl.sub_id(1).unwrap(), &[], (OWNER_STAGE, 0, 0))
            .expect("应能派一个任务");

        // 帧 1：跑到 wait(2) 让出（此时 sh_xform 已设、还没开火）。
        w.body.frame = 1;
        crate::ecl::vm::run_tasks(&mut w.tasks, &mut w.body, &ecl, &TABLES_V0);
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "跨帧前还没开火");
        assert_ne!(
            w.tasks.shooters[idx as usize][0].xform_cnt, 0,
            "前置条件：sh_xform 应已写进槽"
        );
        // 帧 2：wait 递减，本帧不执行。
        w.body.frame = 2;
        crate::ecl::vm::run_tasks(&mut w.tasks, &mut w.body, &ecl, &TABLES_V0);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        // 帧 3：开火——xform 必须仍生效。
        w.body.frame = 3;
        crate::ecl::vm::run_tasks(&mut w.tasks, &mut w.body, &ecl, &TABLES_V0);

        let b = w
            .body
            .bullets
            .iter_alive()
            .next()
            .expect("跨帧后应发出一颗弹");
        let seg = w.body.bullets.transform_head[b];
        assert_ne!(
            seg,
            crate::xform::XFORM_NONE,
            "跨帧后 xform 丢了（延迟读没读到本任务 locals）"
        );
        assert_eq!(
            w.body.xforms.seg_slots(seg)[0],
            XformSlot {
                wait: 3,
                op: crate::xform::OP_SET_SPEED,
                _pad: 0,
                args: [arg0, 0],
            },
            "跨帧后段内容应与设 xform 时的 locals 逐位相同"
        );
    }

    /// 挂弹任务：每颗弹各派一个任务，owner = (BULLET, 该弹 index/gen)。
    #[test]
    fn sh_task_spawns_one_task_per_bullet() {
        let ecl = async_pattern_image(); // sub raw=1 是 0 参 Async
        let mut w = World::new(1);
        w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 3, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_TASK, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

        let bullets: Vec<usize> = w.body.bullets.iter_alive().collect();
        assert_eq!(bullets.len(), 3);
        let spawned: Vec<usize> = (0..crate::ecl::task::TASK_CAP)
            .filter(|&i| w.tasks.is_alive(i))
            .collect();
        assert_eq!(spawned.len(), 3, "每颗弹各派一个任务");
        for (k, (&ti, &bi)) in spawned.iter().zip(&bullets).enumerate() {
            assert_eq!(w.tasks.slots[ti].owner_kind, OWNER_BULLET, "第 {k} 个");
            assert_eq!(w.tasks.slots[ti].owner_index, bi as u16, "第 {k} 个");
            assert_eq!(
                w.tasks.slots[ti].owner_gen, w.body.bullets.generation[bi],
                "第 {k} 个"
            );
        }
    }

    /// P4-a：任务池满 → 弹**保留**、`pool_full[POOL_TASK]` 逐颗计数、不 Fault（同 `fire` 口径）。
    #[test]
    fn sh_task_degrades_when_task_pool_is_full() {
        let ecl = async_pattern_image();
        let mut w = World::new(1);
        // **先灌满任务池、再配 shooter**：`TaskPool::spawn` 的"复用槽写满"纪律会把
        // 每个新槽的 shooter 抹成默认值（含 0 号槽），先配后灌会被抹掉。
        while w
            .tasks
            .spawn(ecl.sub_id(1).unwrap(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .is_some()
        {}
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 3, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_TASK, &[0, 1]);

        let before = w.body.diag.pool_full[crate::world::POOL_TASK];
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            3,
            "任务池满不影响弹——弹保留"
        );
        assert_eq!(
            w.body.diag.pool_full[crate::world::POOL_TASK],
            before + 3,
            "逐颗计一次 pool_full[POOL_TASK]"
        );
    }

    /// `on_fire_req`：发一条请求，`args[3]` 是**实际**创建数而非请求数。
    /// 判别腿：把弹池灌到只剩 2 格再发 5 颗的环，args[3] 必须是 2。
    #[test]
    fn on_fire_req_reports_actual_count_not_requested() {
        let (mut w, ecl) = fresh_with_shooters();
        for _ in 0..(crate::bullets::BulletPool::CAP - 2) {
            w.body.create_bullet(filler_bullet());
        }
        let ox = Fx::from_int(12).raw();
        let oy = Fx::from_int(-34).raw();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_C]);
        sh(&mut w, &ecl, &mut t, SYS_SH_OFFSET, &[0, ox, oy]);
        sh(&mut w, &ecl, &mut t, SYS_SH_RING, &[0, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 5, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_REQ, &[0, 77]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

        assert_eq!(
            w.body.bullets.iter_alive().count(),
            crate::bullets::BulletPool::CAP,
            "只剩 2 格 ⇒ 请求 5 颗只发得出 2 颗"
        );
        let r = *w
            .body
            .take_requests()
            .iter()
            .find(|r| r.id == 77)
            .expect("on_fire_req ≠ 0 应发一条请求");
        assert_eq!(r.args[0], ox, "args[0] = 原点 x");
        assert_eq!(r.args[1], oy, "args[1] = 原点 y");
        assert_eq!(r.args[2], ROW_C, "args[2] = appearance");
        assert_eq!(r.args[3], 2, "args[3] 必须是**实际**创建数（不是请求的 5）");
    }

    /// `on_fire_req == 0` = 不发（默认值的语义，别发一条 id=0 的垃圾请求）。
    #[test]
    fn on_fire_req_zero_emits_nothing() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        assert_eq!(w.body.bullets.iter_alive().count(), 1);
        assert!(
            w.body.take_requests().is_empty(),
            "on_fire_req=0 不该发请求"
        );
    }

    /// P4-b：n_angle 或 n_speed 为 0、或乘积超弹池 CAP → 不发 + contract_viol
    /// （对齐 `create_bullets_batch` 的既有口径）。
    #[test]
    fn sh_fire_rejects_degenerate_grid() {
        const {
            assert!(
                255 * 255 > crate::bullets::BulletPool::CAP,
                "前置条件：255×255 必须真的超弹池 CAP"
            )
        };
        for (n_angle, n_speed, why) in [
            (0, 1, "n_angle=0"),
            (1, 0, "n_speed=0"),
            (255, 255, "超CAP"),
        ] {
            let (mut w, ecl) = fresh_with_shooters();
            let mut t = Task::default();
            sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
            sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, n_angle, n_speed]);
            let viol = w.body.diag.contract_viol;
            w.body.last_status = crate::world::STATUS_OK;
            sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
            assert_eq!(
                w.body.bullets.iter_alive().count(),
                0,
                "{why}：一颗都不该发"
            );
            assert_eq!(
                w.body.diag.contract_viol,
                viol + 1,
                "{why}：应记一次 contract_viol"
            );
            assert_eq!(
                w.body.last_status,
                crate::world::STATUS_BAD_ARGS,
                "{why}：应置 STATUS_BAD_ARGS"
            );
        }
    }

    /// `sh_fire` 的 `id` 越界走**与 14 个 setter 同一口径**：no-op + contract_viol +
    /// STATUS_BAD_ARGS，不 Fault（判据集中在 `shooter_mut` 一处）。
    #[test]
    fn sh_fire_rejects_out_of_range_id() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        for bad in [SHOOTERS_PER_TASK as i32, -1] {
            let viol = w.body.diag.contract_viol;
            w.body.last_status = crate::world::STATUS_OK;
            assert!(
                call(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[bad]).is_ok(),
                "id={bad} 是 no-op 而非 Fault"
            );
            assert_eq!(t.sp, 0, "id={bad} 腿也应把实参弹栈");
            assert_eq!(w.body.bullets.iter_alive().count(), 0, "id={bad} 不该发弹");
            assert_eq!(w.body.diag.contract_viol, viol + 1, "id={bad}");
            assert_eq!(
                w.body.last_status,
                crate::world::STATUS_BAD_ARGS,
                "id={bad}"
            );
        }
    }

    /// appearance 的在册/空格校验**推迟到开火那一刻**（T2 的 14 个 setter 只写字段）——
    /// 越界号与空格都 Fault，且零副作用。这条正是 T2 复审 ① 那条"收窄必须保号越界性"
    /// 的下游消费者：`sh_sprite` 若把负值钳成 0，这里就再也拒不掉了。
    #[test]
    fn sh_fire_faults_on_out_of_range_or_blank_appearance() {
        // ① 越界号（`sh_sprite` 收窄成 u16::MAX，远超表长）
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, -4]);
        assert_eq!(
            call(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]),
            Err(FAULT_BAD_OP),
            "越界 appearance 必须 Fault"
        );
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "先验后建：零副作用");

        // ② 空格（合成表）
        const BLANK: i32 = 3 * 16 + 7;
        let holed = tables_with_hole(BLANK as usize);
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, BLANK]);
        assert_eq!(
            call_with_tables(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0], &holed),
            Err(FAULT_BAD_OP),
            "空格 appearance 必须 Fault（隐形弹）"
        );
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
    }

    /// `SH_ABS_OFFSET`：绝对偏移**不跟随 owner**，相对偏移跟随。
    /// 必须用非 STAGE 的 owner 才判别得了——STAGE 的 `self_pos` 恒 (0,0)，两条分支同结果。
    #[test]
    fn abs_offset_ignores_owner_position_while_relative_follows_it() {
        let dx = Fx::from_int(7).raw();
        let dy = Fx::from_int(9).raw();
        let fire = |abs: bool| -> (i32, i32) {
            let (mut w, ecl) = fresh_with_shooters();
            let eh = crate::world::test_support::spawn_enemy(&mut w, 100, -60, 5);
            let mut t = Task {
                owner_kind: OWNER_ENEMY,
                owner_index: eh.index,
                owner_gen: eh.generation,
                ..Task::default()
            };
            sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
            let no = if abs {
                SYS_SH_OFFSET_ABS
            } else {
                SYS_SH_OFFSET
            };
            sh(&mut w, &ecl, &mut t, no, &[0, dx, dy]);
            sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
            let i = w.body.bullets.iter_alive().next().expect("应发一颗");
            (w.body.bullets.x[i].raw(), w.body.bullets.y[i].raw())
        };
        assert_eq!(fire(true), (dx, dy), "绝对偏移：原点就是偏移量本身");
        assert_eq!(
            fire(false),
            (Fx::from_int(100).raw() + dx, Fx::from_int(-60).raw() + dy),
            "相对偏移：原点 = owner 位置 + 偏移量"
        );
    }

    // ── 数学/查询面（110/140/141；小清洗刀 2026-07-31）────────────────────────────────

    /// **参数序判别腿**：`atan2(y, x)` 两位同为 `Fx`，对调不会有任何判型报错，只会静默
    /// 把角度镜像到另一条对角线上。故必须**两个取值**一起断言——只测 `atan2(1,0)==16384`
    /// 是抓不到"实现写成 `atan2(x, y)`"的（`atan2(0,1)` 那腿才把它钉死）。
    #[test]
    fn atan2_argument_order_is_y_then_x() {
        let (mut w, ecl) = fresh();
        let mut t = Task::default();
        let one = Fx::from_int(1).raw();
        assert!(call(&mut w, &ecl, &mut t, SYS_ATAN2, &[one, 0]).is_ok());
        assert_eq!(t.sp, 1, "atan2 押一个返回值");
        assert_eq!(t.stack[0], 16384, "atan2(y=1, x=0) = +90° = BAM 16384");
        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, SYS_ATAN2, &[0, one]).is_ok());
        assert_eq!(t.stack[0], 0, "atan2(y=0, x=1) = 0° = BAM 0");
    }

    /// `(0,0)` 无 P4 分支——CORDIC 对原点有定义（0），不 Fault、不计违约。
    #[test]
    fn atan2_at_origin_is_zero_without_fault() {
        let (mut w, ecl) = fresh();
        let mut t = Task::default();
        let viol = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut t, SYS_ATAN2, &[0, 0]).is_ok());
        assert_eq!(t.stack[0], 0);
        assert_eq!(w.body.diag.contract_viol, viol, "原点不是违约");
    }

    /// **开根判别腿**：3-4-5 直角三角形。若实现漏了 `isqrt`（直接押 `len_sq`）或把
    /// Q32.32 当 Q16.16 塞回去，这条立刻红——只断言"结果 > 0"是抓不到的
    /// （`len_sq(3,4)` 也 > 0）。
    #[test]
    fn dist_is_the_square_root_of_the_squared_length() {
        let (mut w, ecl) = fresh();
        let mut t = Task::default();
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_DIST,
                &[Fx::from_int(3).raw(), Fx::from_int(4).raw()]
            )
            .is_ok()
        );
        assert_eq!(t.sp, 1, "dist 押一个返回值");
        assert_eq!(
            t.stack[0],
            Fx::from_int(5).raw(),
            "dist(3.0fx, 4.0fx) 必须正好是 5.0fx"
        );
    }

    /// `dist` 是**向量模**（对称、恒非负）——负分量与正分量同结果。
    #[test]
    fn dist_of_negative_components_is_the_same_magnitude() {
        let (mut w, ecl) = fresh();
        let mut t = Task::default();
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_DIST,
                &[Fx::from_int(-3).raw(), Fx::from_int(-4).raw()]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], Fx::from_int(5).raw());
    }

    /// 极端输入的收窄腿（复审 Minor）：`dist` 是**通用两参 syscall**，脚本能直接喂任意
    /// `fx`——安全论证里那句"满屏最大 1.7e15"只覆盖世界坐标差这个域，覆盖不到字面量。
    ///
    /// `dx = dy = i32::MAX`：`len_sq` = 2×(2147483647²) = 9223372028264841218
    /// （**贴着 i64 上限 9223372036854775807 但不溢出**，故 debug 下 `len_sq` 自身不 panic），
    /// `isqrt` = **3037000498** > `i32::MAX`(2147483647) ⇒ 裸 `as i32` 回绕成
    /// **-1257966798**。断言必须钉"等于 `i32::MAX`"而**不是**"结果 ≥ 0"——后者对
    /// "回绕后恰好落在正半区"的输入是瞎的。
    #[test]
    fn dist_saturates_instead_of_wrapping_negative() {
        let (mut w, ecl) = fresh();
        let mut t = Task::default();
        assert!(call(&mut w, &ecl, &mut t, SYS_DIST, &[i32::MAX, i32::MAX]).is_ok());
        assert_eq!(
            t.stack[0],
            i32::MAX,
            "开根超 i32 上限时饱和到最大可表示距离，不得回绕成负数"
        );
    }

    /// **"取最近"判别腿**：场上放**两只**不同距离的敌——只放一只的话，"取最近"与
    /// "取第一个活着的"无法区分（同本仓「圆心重合式测试对半径映射是瞎的」那条推论）。
    /// 更进一步：近的那只**池索引更大**，故"返回最低存活索引"这个变异也被钉死。
    #[test]
    fn nearest_enemy_returns_the_nearer_of_two_enemies() {
        let (mut w, ecl) = fresh();
        let far = crate::world::test_support::spawn_enemy(&mut w, 10, 0, 5);
        let near = crate::world::test_support::spawn_enemy(&mut w, 3, 0, 5);
        assert!(
            near.index > far.index,
            "近敌须是后建的（索引更大）才有判别力"
        );
        let mut t = Task::default();
        assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
        assert_eq!(t.sp, 1, "nearest_enemy 押一个返回值");
        assert_eq!(
            t.stack[0],
            crate::enemy::pack_handle(near),
            "查询点 (0,0) 附近的是近敌"
        );

        // 反过来查：从远敌那侧看，最近的换成远敌——防"恒返回某个固定槽"。
        t.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_NEAREST_ENEMY,
                &[Fx::from_int(20).raw(), 0]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], crate::enemy::pack_handle(far));
    }

    /// 空场 → -1（同 `enemy_hp` 的"查不到押 -1"口径，不 Fault）。
    #[test]
    fn nearest_enemy_on_empty_field_is_minus_one() {
        let (mut w, ecl) = fresh();
        let mut t = Task::default();
        assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
        assert_eq!(t.stack[0], -1);
    }

    /// **轴判别腿**：敌放在 **x ≠ y** 的位置（30, −70）分别断言——放 (5,5) 的话把两条
    /// 派发臂写反是完全看不出来的（同「圆心重合式测试对半径映射是瞎的」那条推论）。
    /// 负坐标那一半还顺带钉死"别把 `Fx` raw 当无符号搬"。
    #[test]
    fn enemy_x_and_enemy_y_read_their_own_axis() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        let mut t = Task::default();
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_X,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.sp, 1, "enemy_x 押一个返回值");
        assert_eq!(
            t.stack[0],
            Fx::from_int(30).raw(),
            "enemy_x 读的是 x（30），不是 y（-70）"
        );

        t.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_Y,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(
            t.stack[0],
            Fx::from_int(-70).raw(),
            "enemy_y 读的是 y（-70），不是 x（30）"
        );
    }

    /// 无效句柄三腿（负 / 越界 / 死槽）一律 **0**（P4-b 纯读族降级，不 Fault、不计
    /// `contract_viol`）。坐标没有 `enemy_hp` 那样的哨兵位可用（−1 是合法 `fx`），故取
    /// `Fx::ZERO`——探活归脚本（`enemy_hp(e) != -1`），手册写死了这条惯例。
    #[test]
    fn enemy_pos_on_invalid_handle_degrades_to_zero() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        let before = w.body.diag.contract_viol;
        let mut t = Task::default();

        for (no, name) in [(SYS_ENEMY_X, "enemy_x"), (SYS_ENEMY_Y, "enemy_y")] {
            t.sp = 0;
            assert!(call(&mut w, &ecl, &mut t, no, &[-1]).is_ok());
            assert_eq!(t.stack[0], 0, "{name}：负句柄降级返 0");

            t.sp = 0;
            assert!(call(&mut w, &ecl, &mut t, no, &[9999]).is_ok());
            assert_eq!(t.stack[0], 0, "{name}：越界句柄降级返 0");
        }

        w.body.enemies.free(h);
        for (no, name) in [(SYS_ENEMY_X, "enemy_x"), (SYS_ENEMY_Y, "enemy_y")] {
            t.sp = 0;
            assert!(call(&mut w, &ecl, &mut t, no, &[crate::enemy::pack_handle(h)]).is_ok());
            assert_eq!(t.stack[0], 0, "{name}：死槽降级返 0");
        }
        assert_eq!(
            w.body.diag.contract_viol, before,
            "纯读族降级不计违约（同 enemy_hp 口径）"
        );
    }

    /// `ENEMY_DYING` 的敌**仍可读**——`is_alive` 是存活位，dying 只是 flag，槽要活到相位 9
    /// 才回收。别顺手把 dying 也降级了，那与 `enemy_hp` 的既有读族口径不一致。
    #[test]
    fn enemy_pos_is_readable_while_dying() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        w.body.enemies.flags[h.index as usize] |= crate::enemy::ENEMY_DYING;
        let mut t = Task::default();
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_X,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], Fx::from_int(30).raw(), "dying 的敌坐标仍读得到");
        t.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_Y,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], Fx::from_int(-70).raw());
    }

    /// owner 类别无限制（同 `enemy_hp`/`nearest_enemy`）：STAGE 任务照样能读。
    #[test]
    fn enemy_pos_is_callable_from_a_stage_task() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        let mut t = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_X,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], Fx::from_int(30).raw());
    }

    // ── 探活读口 103（enemy_alive；探活读口刀 2026-07-31）──────────────────────

    /// 活敌返 **1**、三种无效句柄（负 / 越界 / 死槽）各返 **0**，且**不计 `contract_viol`**
    /// （纯读族口径，同 `enemy_hp`/`sys_enemy_pos`）。
    #[test]
    fn enemy_alive_is_one_for_a_live_enemy_and_zero_for_invalid_handles() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        let before = w.body.diag.contract_viol;
        let mut t = Task::default();

        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_ALIVE,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.sp, 1, "enemy_alive 押一个返回值");
        assert_eq!(t.stack[0], 1, "活敌返 1");

        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_ALIVE, &[-1]).is_ok());
        assert_eq!(t.stack[0], 0, "负句柄返 0");

        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_ALIVE, &[9999]).is_ok());
        assert_eq!(t.stack[0], 0, "越界句柄返 0");

        w.body.enemies.free(h);
        t.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_ALIVE,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], 0, "死槽返 0");

        assert_eq!(
            w.body.diag.contract_viol, before,
            "纯读族降级不计违约（同 enemy_hp 口径）"
        );
    }

    /// **判别腿①——本刀唯一的语义裁定**：`ENEMY_DYING` 的敌 `enemy_alive` 仍返 **1**。
    /// 判的是「槽有效」而不是「还能打」：读族四条（`enemy_hp`/`enemy_x`/`enemy_y`/
    /// `enemy_alive`）必须用**完全相同**的三判据——dying 的槽要活到相位 9（坐标仍读得到），
    /// 四条里单独给一条换判据会让这组口径散掉。
    ///
    /// "排除 dying"是最自然的错法（名字读起来就像"还能打"），而它**只有这条测试逮得住**：
    /// dying 在正常路径上是个转瞬即逝的中间态，e2e 与其余各腿全都照绿。
    /// 顺带钉住与 `enemy_x` 的同判：同一个 dying 句柄，两条口必须给出同一个"槽有效"结论。
    #[test]
    fn enemy_alive_stays_one_while_dying() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        w.body.enemies.flags[h.index as usize] |= crate::enemy::ENEMY_DYING;
        let mut t = Task::default();

        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_ALIVE,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(
            t.stack[0], 1,
            "判的是「槽有效」不是「还能打」——dying 的敌仍返 1"
        );

        t.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_X,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(
            t.stack[0],
            Fx::from_int(30).raw(),
            "坐标读口对 dying 同判（读族四条判据逐字一致）"
        );
    }

    /// **判别腿②——与 `enemy_hp` 的判据一致性**：同一个句柄，`enemy_alive == 1` ⟺
    /// `enemy_hp` **不因"槽无效"**降级。三种无效（负 / 越界 / 死槽）各断言两者同步
    /// （`0` ⟺ `-1`）。
    ///
    /// 外加本 syscall **存在的全部理由**那一格：活敌血量**恰为 −1** 时（overkill 的敌 hp
    /// 是真实负值，`settle::kill_enemy` 只 `min(0)` 不抹平）`enemy_hp` 返 −1，与降级值
    /// 撞车——旧探针 `enemy_hp(e) != -1` 就在这一格误判，而 `enemy_alive` 照返 1。
    /// 这一格红了就说明新口只是 `enemy_hp` 的花哨包装、白加一号。
    #[test]
    fn enemy_alive_agrees_with_enemy_hp_and_closes_the_minus_one_seam() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        let mut t = Task::default();

        // ① 缝本身：活敌的 hp 恰好撞上降级哨兵 −1。
        w.body.enemies.hp[h.index as usize] = -1;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_HP,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], -1, "旧探针的盲区：活敌 hp 与降级值不可辨");
        t.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_ALIVE,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], 1, "槽有效 ⇒ 1（这一格是本刀存在的全部理由）");

        // ② 三种无效：两条口必须同步（alive=0 ⟺ hp 降级成 −1）。
        w.body.enemies.free(h);
        for (handle, name) in [
            (-1i32, "负句柄"),
            (9999, "越界"),
            (crate::enemy::pack_handle(h), "死槽"),
        ] {
            t.sp = 0;
            assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_ALIVE, &[handle]).is_ok());
            let alive = t.stack[0];
            t.sp = 0;
            assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_HP, &[handle]).is_ok());
            let hp = t.stack[0];
            assert_eq!((alive, hp), (0, -1), "{name}：两条口同步降级");
        }
    }

    /// owner 类别无限制（同 `enemy_hp`/`enemy_x`/`nearest_enemy`）：STAGE 任务照样能探。
    #[test]
    fn enemy_alive_is_callable_from_a_stage_task() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
        let mut t = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(
            call(
                &mut w,
                &ecl,
                &mut t,
                SYS_ENEMY_ALIVE,
                &[crate::enemy::pack_handle(h)]
            )
            .is_ok()
        );
        assert_eq!(t.stack[0], 1);
    }

    /// owner 类别无限制：STAGE 任务（关卡编排）照样能查。
    #[test]
    fn nearest_enemy_is_callable_from_a_stage_task() {
        let (mut w, ecl) = fresh();
        let h = crate::world::test_support::spawn_enemy(&mut w, 5, 5, 3);
        let mut t = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
        assert_eq!(t.stack[0], crate::enemy::pack_handle(h));
    }

    // ── 敌句柄打包 generation（敌句柄打包刀 2026-07-31）─────────────────────────

    /// 用 `spawn_enemy`(210) 造敌并取**脚本视角**的敌号——即 syscall 真正押出去的那个值。
    /// 本族测试必须走 syscall 边界：被测的就是那个边界上的编码，绕过它（直接拿
    /// `test_support::spawn_enemy` 的 `EnemyHandle`）就把要证的东西假设掉了。
    fn spawn_enemy_via_syscall(w: &mut World, ecl: &EclImage, x: i32, y: i32, hp: i32) -> i32 {
        let mut t = Task::default();
        // 正序：x,y,hp,drop_table,score,sprite,task(none=-1)
        let args = [
            Fx::from_int(x).raw(),
            Fx::from_int(y).raw(),
            hp,
            0,
            100,
            0,
            -1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(w, ecl, &mut t, SYS_SPAWN_ENEMY, &args).is_ok());
        t.stack[0]
    }

    /// 单参读口的一次调用（新 `Task`，返回押回的那一个值）。
    fn read_enemy_port(w: &mut World, ecl: &EclImage, no: u16, handle: i32) -> i32 {
        let mut t = Task::default();
        assert!(call(w, ecl, &mut t, no, &[handle]).is_ok());
        assert_eq!(t.sp, 1, "读口押且只押一个返回值");
        t.stack[0]
    }

    /// **这刀存在的全部理由 —— ABA**：敌 A 死、槽被 cleanup 回收、敌 B 落进**同一个槽**
    /// 之后，A 的**旧句柄**必须读到降级值，而不是静默变成 B 的号读到 B 的数据。
    ///
    /// 改动前这条是红的（敌号是裸池 index，A/B 同槽 ⇒ 两者的号逐位相同），红的正是
    /// `enemy_hp` 读到 B 的血那一格——问题真实存在的实证。
    ///
    /// 判别力靠三件事：① B 必须落进 A 的**同一个槽**（分配器取最低空位，故先断言 A 占
    /// 0 号槽、回收后最低空位仍是 0），否则整条测试什么也没证明；② A/B 的坐标与血量
    /// **全部不同**，读到 B 必然可辨；③ 末尾用 B 的**新**句柄再读一遍四个口——防
    /// "resolve 恒失败、全都读不到"那种假绿。
    #[test]
    fn a_stale_enemy_handle_does_not_read_the_enemy_that_took_its_slot() {
        let (mut w, ecl) = fresh();

        // A：空池 ⇒ 落最低空位（0 号槽）。
        let a = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
        let a_idx = (a & 0xFFFF) as usize;
        assert_eq!(a_idx, 0, "前提：A 占最低空位（否则 B 未必落回同一个槽）");

        // A 死 + 槽被相位 9 回收（走完整 step，不手 `free`——回收纪律本身也在链路里）。
        w.body.enemies.flags[a_idx] |= crate::enemy::ENEMY_DYING;
        let frame = w.body.frame;
        crate::step::step(
            &mut w,
            &TABLES_V0,
            &ecl,
            &crate::input::InputFrame::empty(frame),
        );
        assert!(
            !w.body.enemies.is_alive(a_idx),
            "前提：A 的槽已被 cleanup 回收"
        );

        // B：最低空位仍是 0 ⇒ 落进 A 的旧槽。坐标/血量与 A 全不同。
        let b = spawn_enemy_via_syscall(&mut w, &ecl, 55, 66, 123);
        assert_eq!(
            (b & 0xFFFF) as usize,
            a_idx,
            "前提：B 必须落进 A 的旧槽，否则本测试什么都没证明"
        );

        // 用 A 的**旧句柄**读四个口：全部降级，不得读到 B 的任何一个字段。
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, a),
            -1,
            "旧句柄的 enemy_hp 必须降级 —— 读到 123 就是读到了 B 的血（ABA）"
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, a),
            0,
            "旧句柄的 enemy_x 必须降级 —— 读到 55 就是 B 的 x"
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, a),
            0,
            "旧句柄的 enemy_y 必须降级 —— 读到 66 就是 B 的 y"
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, a),
            0,
            "旧句柄的 enemy_alive 必须返 0 —— A 已经不在了"
        );

        assert_ne!(
            b, a,
            "同槽而不同敌 ⇒ 两个句柄必须可辨（generation 就是干这个的）"
        );

        // 反向腿（防"全都读不到"的假绿）：B 的**新**句柄一切正常。
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, b), 123);
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, b),
            Fx::from_int(55).raw()
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, b),
            Fx::from_int(66).raw()
        );
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, b), 1);

        // `nearest_enemy` 押的必须是**同一套编码**（配对使用：拿号 → 轮询）。
        let mut t = Task::default();
        assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
        assert_eq!(
            t.stack[0], b,
            "nearest_enemy 与 spawn_enemy 同口径（含 gen）"
        );
    }

    /// **掩码腿**：`generation` 是完整 `u16` 而句柄只带低 15 位，故比对两边都要
    /// `& 0x7FFF`。gen 高位置位（`0xF00D`）的敌，其句柄必须 ① 仍然**非负**
    /// （否则 `-1` 哨兵不再唯一）、② 喂回四个读口全部照常工作。
    ///
    /// 漏掉掩码的实现在这里红：`generation[idx] == g` 会拿 `0xF00D` 比 `0x700D`，
    /// 于是"敌活着但所有读口都说它没了"——正常路径上要跑 32768 次同槽复用才撞得到，
    /// 只有这条测试逮得住。
    #[test]
    fn packed_handle_stays_non_negative_and_resolves_with_a_high_generation() {
        let (mut w, ecl) = fresh();
        // alloc 会把槽的 gen +1，故预置 0xF00C ⇒ 敌的 generation = 0xF00D（最高位置位）。
        w.body.enemies.generation[0] = 0xF00C;
        let h = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
        assert_eq!(
            w.body.enemies.generation[0], 0xF00D,
            "前提：这只敌的 generation 最高位已置位"
        );

        assert!(
            h >= 0,
            "打包值必须恒非负 —— 只押 gen 的低 15 位就是为了这个"
        );
        assert_eq!((h & 0xFFFF) as usize, 0, "低 16 位仍是池 index");
        assert_eq!(
            (h >> 16) & 0x7FFF,
            0xF00D & 0x7FFF,
            "高位押的是 generation 的低 15 位"
        );

        // 往返：打包出来的句柄喂回四个读口全部工作（比对侧漏掩码则四条全红）。
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, h), 77);
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, h),
            Fx::from_int(30).raw()
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, h),
            Fx::from_int(-70).raw()
        );
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, h), 1);
    }

    /// 正常路径不变：`spawn_enemy` 押的号直接喂四个读口全部工作（打包不该把"刚建好的敌
    /// 读不到"当代价）。顺带钉住 `nearest_enemy` 返的就是同一个值。
    #[test]
    fn freshly_spawned_enemy_handle_feeds_all_four_read_ports() {
        let (mut w, ecl) = fresh();
        let h = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
        assert!(h >= 0, "敌应建成");
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, h), 77);
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, h),
            Fx::from_int(30).raw()
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, h),
            Fx::from_int(-70).raw()
        );
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, h), 1);

        let mut t = Task::default();
        assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
        assert_eq!(t.stack[0], h, "两条产号口必须同编码");
    }

    /// `-1` 仍是四个读口眼里的唯一无效哨兵——打包后**不得**有哪个 `-1` 意外解包成合法槽。
    ///
    /// ⚠️ 注意这条**打不中** `resolve_enemy_handle` 的 `packed >= 0` 那道闸：`-1` 的低 16 位
    /// 是 `0xFFFF` = 65535，越界判据自己就兜住了。专打非负闸的判别腿见下一条。
    #[test]
    fn minus_one_is_still_invalid_for_every_read_port() {
        let (mut w, ecl) = fresh();
        // 场上放一只活敌：空场的话"恒降级"的实现也会绿。
        let live = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, live), 1);

        let before = w.body.diag.contract_viol;
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, -1), -1);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, -1), 0);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, -1), 0);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, -1), 0);
        assert_eq!(
            w.body.diag.contract_viol, before,
            "纯读族降级仍不计违约（口径未变）"
        );
    }

    /// **非负闸的判别腿**（复审 ②）：`resolve_enemy_handle` 的 `packed >= 0` 那道闸此前
    /// 是**被越界判据遮住的**——常见负值（`-1`）的低 16 位是 `0xFFFF` = 65535 ≥ CAP(256)，
    /// 删掉非负闸测试照绿。今天没事，但将来谁放宽越界判据或扩了敌池容量，洞就露出来。
    ///
    /// 能单独打中它的取值是 **`-65536`**（`0xFFFF0000`）：低 16 位 = `0`（合法槽），
    /// `(p >> 16) & 0x7FFF` = `0x7FFF`（算术右移把符号位铺满）。拿它去打一只
    /// `generation & 0x7FFF == 0x7FFF` 的**活敌**——index 合法、gen 也对得上，
    /// **只有非负闸能拒绝它**。
    ///
    /// 反向腿钉住这不是"什么都读不到"：同一只敌的**正**句柄 `0x7FFF0000` 必须照常读通。
    #[test]
    fn a_negative_handle_whose_low_bits_alias_a_live_enemy_is_still_rejected() {
        let (mut w, ecl) = fresh();
        // alloc 会 +1，故预置 0x7FFE ⇒ 这只敌的 generation = 0x7FFF（低 15 位全 1）。
        w.body.enemies.generation[0] = 0x7FFE;
        let good = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
        assert_eq!(
            w.body.enemies.generation[0], 0x7FFF,
            "前提：gen 的低 15 位必须全 1，否则 -65536 解出来的 gen 对不上、本腿失去判别力"
        );
        assert_eq!(good, 0x7FFF_0000, "前提：正句柄就是 -65536 的非负孪生");

        // 反向腿：正句柄照常读通（否则下面四条退化成"什么都读不到"的假绿）。
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, good), 77);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, good), 1);

        // 正题：同 index、同 gen，只差一个符号位 —— 必须被非负闸拒掉。
        const ALIAS: i32 = -65536; // 0xFFFF0000
        assert_eq!(ALIAS & 0xFFFF, 0, "低 16 位确实指向 0 号槽（活敌）");
        assert_eq!(
            (ALIAS >> 16) & 0x7FFF,
            0x7FFF,
            "解出的 gen 确实与那只敌相符"
        );
        assert_eq!(
            read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, ALIAS),
            -1,
            "负句柄必须被拒 —— 删掉 `packed >= 0` 这条就红"
        );
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, ALIAS), 0);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, ALIAS), 0);
        assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, ALIAS), 0);
    }

    // ── 表现契约 v2（2026-09-07）：430 set_anm_state / 721 fx_at / 722 fx_on / born_frame ──

    /// 430：写状态并盖帧；**同状态重设也盖**（= 重播，ZUN interrupt 重触发语义）。
    #[test]
    fn set_anm_state_restamps_frame_even_for_same_state() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        let i = eh.index as usize;
        w.body.frame = 10;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ANM_STATE, &[3]).is_ok());
        assert_eq!(w.body.enemies.anm_state[i], 3);
        assert_eq!(w.body.enemies.anm_state_frame[i], 10);
        w.body.frame = 25;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ANM_STATE, &[3]).is_ok());
        assert_eq!(w.body.enemies.anm_state[i], 3, "状态不变");
        assert_eq!(w.body.enemies.anm_state_frame[i], 25, "帧号必须重盖");
        assert_eq!(w.body.diag.contract_viol, 0);
    }

    /// 430 / 722 self-only：owner 非 ENEMY → Fault（同 400 族误用策略）；栈不留垃圾。
    #[test]
    fn set_anm_state_and_fx_on_fault_on_non_enemy_owner() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default(); // STAGE owner
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_SET_ANM_STATE, &[1]),
            Err(FAULT_BAD_OP)
        );
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_FX_ON, &[1, 2]),
            Err(FAULT_BAD_OP)
        );
        assert!(w.body.take_requests().is_empty(), "Fault 路径不得发请求");
    }

    /// 721：布局钉死 `REQ_FX_AT, [x raw, y raw, kind, param, 0, 0]`；owner 无限制。
    #[test]
    fn fx_at_emits_pinned_request_layout() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = [Fx::from_int(12).raw(), Fx::from_int(-34).raw(), 7, 99];
        assert!(call(&mut w, &ecl, &mut task, SYS_FX_AT, &args).is_ok());
        let r = w.body.take_requests();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, crate::consts::REQ_FX_AT);
        assert_eq!(
            r[0].args,
            [Fx::from_int(12).raw(), Fx::from_int(-34).raw(), 7, 99, 0, 0]
        );
        assert_eq!(task.sp, 0, "无返回值，栈应回到空");
    }

    /// 722：布局钉死 `REQ_FX_ATTACHED, [index, gen, kind, param, 0, 0]`，句柄取自 owner。
    #[test]
    fn fx_on_emits_owner_handle_as_two_raw_slots() {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, SYS_FX_ON, &[5, -8]).is_ok());
        let r = w.body.take_requests();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, crate::consts::REQ_FX_ATTACHED);
        assert_eq!(
            r[0].args,
            [eh.index as i32, eh.generation as i32, 5, -8, 0, 0]
        );
    }

    /// 弹的三条创建路径都盖 `born_frame = 创建帧`（spec §3.1）：fire(200) / batch(201) /
    /// sh_fire(660)。帧号取 42 而非 0——零初始化的槽也是 0，0 无判别力。
    #[test]
    fn all_three_bullet_creation_paths_stamp_born_frame() {
        let (mut w, ecl) = fresh();
        w.body.frame = 42;
        // fire
        let mut task = Task::default();
        let args = [ROW_B, 0, 0, Fx::from_int(1).raw(), 0, 0, 0, -1];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
        let i = task.stack[0] as usize;
        assert_eq!(w.body.bullets.born_frame[i], 42, "fire 路径");
        // batch：3 颗
        let mut task = Task::default();
        let n0 = w.body.bullets.iter_alive().count();
        // 正序：appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step
        let args = [ROW_B, 0, 0, 3, 0, 1000, 1, Fx::from_int(1).raw(), 0];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
        assert!(w.body.bullets.iter_alive().count() > n0, "batch 至少造了弹");
        for j in w.body.bullets.iter_alive() {
            assert_eq!(w.body.bullets.born_frame[j], 42, "batch 路径槽 {j}");
        }
        // sh_fire：reset 槽 0 → sprite ROW_B → 开火
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_RESET, &[0]).is_ok());
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, ROW_B]).is_ok());
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, 2, 1]).is_ok());
        let n1 = w.body.bullets.iter_alive().count();
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_FIRE, &[0]).is_ok());
        assert!(
            w.body.bullets.iter_alive().count() > n1,
            "sh_fire 至少造了弹"
        );
        for j in w.body.bullets.iter_alive() {
            assert_eq!(w.body.bullets.born_frame[j], 42, "sh_fire 路径槽 {j}");
        }
    }

    /// `spawn_enemy`（210）盖 `anm_state_frame = 创建帧`、`anm_state = 0`。
    #[test]
    fn spawn_enemy_stamps_anm_state_frame() {
        let (mut w, ecl) = fresh();
        w.body.frame = 42;
        let mut task = Task::default();
        let args = [
            Fx::from_int(5).raw(),
            Fx::from_int(6).raw(),
            42,
            1,
            100,
            0,
            -1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let i = enemy_slot(task.stack[0]);
        assert_eq!(w.body.enemies.anm_state[i], 0);
        assert_eq!(w.body.enemies.anm_state_frame[i], 42);
    }

    // ── boss 换段与敌人钩子刀 Task 2：敌判定族 440-443 / $self_enemy 026 / 清场 531 ──────────

    /// 440 `set_invuln`：写入；`-1`/`65536` 越界整条 no-op + 违约；非敌 owner Fault。
    #[test]
    fn set_invuln_writes_rejects_out_of_range_and_faults_for_non_enemy() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        let i = eh.index as usize;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_INVULN, &[120]).is_ok());
        assert_eq!(w.body.enemies.invuln[i], 120);
        for bad in [-1, 65536] {
            let v0 = w.body.diag.contract_viol;
            assert!(call(&mut w, &ecl, &mut task, SYS_SET_INVULN, &[bad]).is_ok());
            assert_eq!(w.body.enemies.invuln[i], 120, "越界 {bad} 整条 no-op");
            assert_eq!(w.body.diag.contract_viol, v0 + 1);
        }
        let mut stage = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut stage, SYS_SET_INVULN, &[1]),
            Err(FAULT_BAD_OP)
        );
    }

    /// 441/442：各写各的半径，互不串位；负半径钳 0 并计违约。
    #[test]
    fn set_hitbox_and_hurtbox_write_their_own_radius_and_clamp() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        let i = eh.index as usize;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SET_HITBOX,
                &[Fx::from_int(40).raw()]
            )
            .is_ok()
        );
        assert_eq!(w.body.enemies.radius[i], Fx::from_int(40));
        assert_eq!(w.body.enemies.hurtbox[i], Fx::from_int(16));
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SET_HURTBOX,
                &[Fx::from_int(24).raw()]
            )
            .is_ok()
        );
        assert_eq!(w.body.enemies.hurtbox[i], Fx::from_int(24));
        assert_eq!(w.body.enemies.radius[i], Fx::from_int(40));
        let v0 = w.body.diag.contract_viol;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SET_HITBOX,
                &[Fx::from_int(-5).raw()]
            )
            .is_ok()
        );
        assert_eq!(w.body.enemies.radius[i], Fx::ZERO);
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
    }

    /// 443 `set_enemy_flag`：只收 NO_BODY|KILLALL_EXEMPT 的非空子集；含 DYING/未知位/0 整条拒。
    #[test]
    fn set_enemy_flag_accepts_only_settable_bits() {
        use crate::enemy::{ENEMY_DYING, ENEMY_KILLALL_EXEMPT, ENEMY_NO_BODY};
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        let i = eh.index as usize;
        let (nb, ex) = (ENEMY_NO_BODY as i32, ENEMY_KILLALL_EXEMPT as i32);
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[nb | ex, 1]).is_ok());
        assert_eq!(
            w.body.enemies.flags[i],
            ENEMY_NO_BODY | ENEMY_KILLALL_EXEMPT
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[ex, 0]).is_ok());
        assert_eq!(w.body.enemies.flags[i], ENEMY_NO_BODY);
        for bad in [0, ENEMY_DYING as i32, 8, nb | ENEMY_DYING as i32] {
            let v0 = w.body.diag.contract_viol;
            assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[bad, 1]).is_ok());
            assert_eq!(
                w.body.enemies.flags[i], ENEMY_NO_BODY,
                "坏掩码 {bad} 不改 flags"
            );
            assert_eq!(w.body.diag.contract_viol, v0 + 1);
        }
    }

    /// 026 `$self_enemy`：敌 owner 押打包敌号；非敌押 -1（打包值 0 合法，故不读 0）。
    #[test]
    fn self_enemy_pushes_packed_handle_or_minus_one() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ENEMY, &[]).is_ok());
        assert_eq!(task.stack[0], crate::enemy::pack_handle(eh));
        let mut stage = Task::default();
        assert!(call(&mut w, &ecl, &mut stage, SYS_SELF_ENEMY, &[]).is_ok());
        assert_eq!(stage.stack[0], -1);
    }

    /// 531 静默模式：跳过调用者 / 免清 / 已死；被杀者无事件、无加分、无掉落。
    #[test]
    fn kill_all_enemies_silent_skips_caller_exempt_and_dying() {
        use crate::enemy::{ENEMY_DYING, ENEMY_KILLALL_EXEMPT, KILL_SILENT};
        let (mut w, ecl) = fresh();
        let (caller, mut task) = enemy_owner_task(&mut w, 10, 0);
        let exempt = crate::world::test_support::spawn_enemy(&mut w, 10, 0, 10);
        let dying = crate::world::test_support::spawn_enemy(&mut w, 20, 0, 10);
        let normal = crate::world::test_support::spawn_enemy(&mut w, 30, 0, 10);
        w.body.enemies.flags[exempt.index as usize] |= ENEMY_KILLALL_EXEMPT;
        w.body.enemies.flags[dying.index as usize] |= ENEMY_DYING;
        load_drop_table_1(&mut w, normal);
        let score0 = w.body.players[0].score;
        let ev0 = w.body.frame_events_len;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_KILL_ALL_ENEMIES,
                &[KILL_SILENT as i32]
            )
            .is_ok()
        );
        let is_dying =
            |w: &World, h: EnemyHandle| w.body.enemies.flags[h.index as usize] & ENEMY_DYING != 0;
        assert!(!is_dying(&w, caller), "调用者自己不杀");
        assert!(!is_dying(&w, exempt), "免清位不杀");
        assert!(is_dying(&w, normal));
        assert_eq!(w.body.players[0].score, score0, "静默不加分");
        assert_eq!(w.body.frame_events_len, ev0, "静默不发事件");
        assert_eq!(w.body.items.iter_alive().count(), 0, "静默不掉落");
    }

    /// 531 击破模式走 die() 全套；坏 mode 整条 no-op + 违约。STAGE owner 无调用者豁免。
    #[test]
    fn kill_all_enemies_die_mode_runs_full_death_and_bad_mode_is_noop() {
        use crate::enemy::{ENEMY_DYING, KILL_DIE};
        use crate::events::EVT_ENEMY_DIED;
        let (mut w, ecl) = fresh();
        let e = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 10);
        load_drop_table_1(&mut w, e);
        let mut stage = Task::default();
        let v0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut stage, SYS_KILL_ALL_ENEMIES, &[2]).is_ok());
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
        assert_eq!(
            w.body.enemies.flags[e.index as usize] & ENEMY_DYING,
            0,
            "坏 mode 不杀"
        );

        let score0 = w.body.players[0].score;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut stage,
                SYS_KILL_ALL_ENEMIES,
                &[KILL_DIE as i32]
            )
            .is_ok()
        );
        assert_ne!(w.body.enemies.flags[e.index as usize] & ENEMY_DYING, 0);
        assert_eq!(
            w.body.players[0].score,
            score0 + 100,
            "test_support 敌 score=100"
        );
        assert_eq!(
            w.body.frame_events[w.body.frame_events_len as usize - 1].kind,
            EVT_ENEMY_DIED
        );
        assert_eq!(w.body.items.iter_alive().count(), 3, "掉落表 1 = 3 颗");
    }

    // ── boss 换段与敌人钩子刀 Task 3：spell_result 131 / clear_bullets_at 541 ──────────

    /// 131 `spell_result`：读槽值；越界 / 负槽号押 0 + 违约。owner 无限制（STAGE 可读）。
    #[test]
    fn spell_result_reads_slot_and_rejects_out_of_range() {
        let (mut w, ecl) = fresh();
        w.body.spell_last_result[1] = crate::spell::SPELL_END_TIMEOUT;
        let mut stage = Task::default();
        assert!(call(&mut w, &ecl, &mut stage, SYS_SPELL_RESULT, &[1]).is_ok());
        assert_eq!(stage.stack[0], crate::spell::SPELL_END_TIMEOUT as i32);
        for bad in [-1, crate::boss::MAX_BOSSES as i32] {
            let mut t = Task::default();
            let v0 = w.body.diag.contract_viol;
            assert!(call(&mut w, &ecl, &mut t, SYS_SPELL_RESULT, &[bad]).is_ok());
            assert_eq!(t.stack[0], 0);
            assert_eq!(w.body.diag.contract_viol, v0 + 1);
        }
    }

    /// 541 `clear_bullets_at`：按参数铺一帧清弹区；stars=0 带 FIELD_NO_STAR。
    #[test]
    fn clear_bullets_at_creates_one_frame_field_with_star_bit() {
        use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
        let (mut w, ecl) = fresh();
        let mut stage = Task::default();
        let args = [
            Fx::from_int(-30).raw(),
            Fx::from_int(200).raw(),
            Fx::from_int(40).raw(),
            0,
        ];
        assert!(call(&mut w, &ecl, &mut stage, SYS_CLEAR_BULLETS_AT, &args).is_ok());
        let f = w.body.fields.iter_alive().next().expect("应建清弹区");
        assert_eq!(w.body.fields.x[f], Fx::from_int(-30));
        assert_eq!(w.body.fields.y[f], Fx::from_int(200));
        assert_eq!(w.body.fields.radius[f], Fx::from_int(40));
        assert_eq!(w.body.fields.life[f], 1);
        assert_eq!(w.body.fields.dmg_per_frame[f], 0);
        assert_eq!(w.body.fields.flags[f], FIELD_CLEAR_BULLETS | FIELD_NO_STAR);

        let (mut w2, _) = fresh();
        let mut args2 = args;
        args2[3] = 1;
        let mut stage2 = Task::default();
        assert!(call(&mut w2, &ecl, &mut stage2, SYS_CLEAR_BULLETS_AT, &args2).is_ok());
        let f2 = w2.body.fields.iter_alive().next().unwrap();
        assert_eq!(w2.body.fields.flags[f2], FIELD_CLEAR_BULLETS);
    }

    // ── boss 换段与敌人钩子刀 Task 4：spawn_enemy 带参（210 调用约定）──────────────

    /// 1 参 Async sub（raw=1）的镜像——spawn_enemy 带参测试专用。
    fn one_arg_async_image() -> EclImage {
        use crate::ecl::image::EclValueType;
        test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![EclValueType::Int]),
            ],
            vec![EntryInit::new("zako", 1)],
            Some(0),
        )
    }

    /// 带参生成（spec §4.2）：同一帧两只敌各拿自己的实参——globals 顶替做不到的判别腿。
    #[test]
    fn spawn_enemy_with_args_gives_each_task_its_own_args_same_frame() {
        let ecl = one_arg_async_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        for v in [111, 222] {
            let args = [0, Fx::from_int(80).raw(), 10, 0, 0, 0, 1, v, 1];
            assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        }
        let got: Vec<i32> = (0..crate::ecl::task::TASK_CAP)
            .filter(|&i| w.tasks.is_alive(i))
            .map(|i| w.tasks.slots[i].locals[0])
            .collect();
        assert_eq!(got, vec![111, 222]);
    }

    /// 带参门禁：个数不符 / none 带参 → Fault(0) 且敌未建；argc 超 LOCALS 或栈不够 → Fault(2)。
    #[test]
    fn spawn_enemy_with_args_rejects_bad_arity_before_creating_enemy() {
        let ecl = one_arg_async_image();
        let cases: [(&[i32], u8); 4] = [
            (&[0, 0, 10, 0, 0, 0, 1, 0], FAULT_BAD_OP), // 1 参 sub 给 0 参
            (&[0, 0, 10, 0, 0, 0, -1, 7, 1], FAULT_BAD_OP), // none 带参
            (&[0, 0, 10, 0, 0, 0, 1, 65], FAULT_STACK), // argc > LOCALS
            (&[0, 0, 10, 0, 0, 0, 1, 3], FAULT_STACK),  // 栈里不够 argc+7
        ];
        for (args, fault) in cases {
            let mut w = World::new(1);
            let mut task = Task::default();
            assert_eq!(
                call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, args),
                Err(fault),
                "{args:?}"
            );
            assert_eq!(
                w.body.enemies.iter_alive().count(),
                0,
                "{args:?}：敌不得建出"
            );
        }
    }

    // ── 8xx 激光族（Task 4；spec 2026-09-25-laser-pool-design §5）────────────────

    /// 一条字段全零的 `LaserInit`（池满测试用；`LaserInit` 是 exhaustive、无 `Default`）。
    fn zero_laser_init() -> crate::lasers::LaserInit {
        crate::lasers::LaserInit {
            ox: Fx::ZERO,
            oy: Fx::ZERO,
            angle: Angle::ZERO,
            omega: 0,
            start: Fx::ZERO,
            end: Fx::ZERO,
            start_len: Fx::ZERO,
            speed: Fx::ZERO,
            width: Fx::ZERO,
            sprite: 0,
            warn: 0,
            active: 1,
            fade: 0,
            timer: 0,
            state: 0,
            anchor_idx: crate::lasers::ANCHOR_NONE,
            anchor_gen: 0,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            dang: 0,
            px: Fx::ZERO,
            py: Fx::ZERO,
            pang: Angle::ZERO,
            flags: 0,
            born_frame: 0,
        }
    }

    /// `laser()` 的 9 参正序：`color,x,y,angle,len,width,warn,active,fade`。
    #[allow(clippy::too_many_arguments)]
    fn laser_args(
        color: i32,
        x: Fx,
        y: Fx,
        angle: Angle,
        len: Fx,
        width: Fx,
        warn: i32,
        active: i32,
        fade: i32,
    ) -> [i32; 9] {
        [
            color,
            x.raw(),
            y.raw(),
            angle.raw() as i32,
            len.raw(),
            width.raw(),
            warn,
            active,
            fade,
        ]
    }

    /// 打包激光句柄 → 池槽下标。
    fn laser_slot(packed: i32) -> usize {
        (packed & 0xFFFF) as usize
    }

    /// `laser()` 返回打包句柄；解码后字段逐项正确（形态一：start=0、end=start_len=len、
    /// speed=0、sprite=color、omega=0）。
    #[test]
    fn laser_create_packs_handle_and_fills_fields() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = laser_args(
            3,
            Fx::from_int(10),
            Fx::from_int(20),
            Angle(16384),
            Fx::from_int(500),
            Fx::from_int(32),
            30,
            120,
            16,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
        let packed = task.stack[0];
        assert!(packed >= 0, "创建成功压回打包句柄");
        let i = laser_slot(packed);
        assert!(w.body.lasers.is_alive(i), "打包句柄应指向活槽");
        assert_eq!(
            (packed >> 16) & 0x7FFF,
            (w.body.lasers.generation[i] & 0x7FFF) as i32,
            "打包句柄只带池代际的低 15 位"
        );
        let l = &w.body.lasers;
        assert_eq!(l.ox[i], Fx::from_int(10));
        assert_eq!(l.oy[i], Fx::from_int(20));
        assert_eq!(l.angle[i], Angle(16384));
        assert_eq!(l.start[i], Fx::ZERO, "start = 0");
        assert_eq!(l.end[i], Fx::from_int(500), "end = len");
        assert_eq!(l.start_len[i], Fx::from_int(500), "start_len = len");
        assert_eq!(l.speed[i], Fx::ZERO, "speed = 0");
        assert_eq!(l.omega[i], 0, "omega = 0");
        assert_eq!(l.width[i], Fx::from_int(32));
        assert_eq!(l.sprite[i], 3, "sprite = color");
        assert_eq!(l.warn[i], 30);
        assert_eq!(l.active[i], 120);
        assert_eq!(l.fade[i], 16);
        assert_eq!(l.flags[i], 0);
    }

    /// `color ∉ 0..=15` → `FAULT_BAD_OP`，先验后建（不建半成品）。
    #[test]
    fn laser_bad_color_faults_without_creating() {
        for color in [-1, 16] {
            let (mut w, ecl) = fresh();
            let mut task = Task::default();
            let args = laser_args(
                color,
                Fx::ZERO,
                Fx::ZERO,
                Angle::ZERO,
                Fx::ZERO,
                Fx::ZERO,
                0,
                0,
                0,
            );
            assert_eq!(
                call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args),
                Err(FAULT_BAD_OP),
                "color={color} 应 Fault"
            );
            assert_eq!(w.body.lasers.iter_alive().count(), 0, "先验后建：零副作用");
        }
    }

    /// 池满 → 押 -1，不 Fault，`pool_full[POOL_LASER]` 加 1。
    #[test]
    fn laser_pool_full_pushes_neg1_and_counts() {
        let (mut w, ecl) = fresh();
        for k in 0..crate::lasers::LaserPool::CAP {
            assert_ne!(
                w.body.create_laser(zero_laser_init()),
                crate::lasers::LaserHandle::NULL,
                "第 {k} 条应成功"
            );
        }
        let mut task = Task::default();
        let pf0 = w.body.diag.pool_full[crate::world::POOL_LASER];
        let args = laser_args(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::ZERO,
            Fx::ZERO,
            0,
            0,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
        assert_eq!(task.stack[0], -1, "池满押 -1，不 Fault");
        assert_eq!(
            w.body.diag.pool_full[crate::world::POOL_LASER],
            pf0 + 1,
            "池满计一次"
        );
    }

    /// **只覆盖时间字段**：`warn/active/fade` 出 `[0,65535]` → 钳位；这三个字段一起越界时
    /// `laser()` 只计一次违约。几何字段（坐标/长度/宽度）另由 `world::create_laser` 再计一次，
    /// 故"时间 + 几何同时坏"的一次调用会合计 2 次——控制方裁定保持现状，本测试不覆盖那半边。
    #[test]
    fn laser_time_fields_clamp_out_of_u16_and_count_once() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let cv0 = w.body.diag.contract_viol;
        let args = laser_args(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::ZERO,
            Fx::ZERO,
            100000,
            -1,
            70000,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
        let i = laser_slot(task.stack[0]);
        assert_eq!(w.body.lasers.warn[i], u16::MAX, "warn 上钳");
        assert_eq!(w.body.lasers.active[i], 0, "active 负值下钳 0");
        assert_eq!(w.body.lasers.fade[i], u16::MAX, "fade 上钳");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "一次 create 只计一次");
    }

    /// `lz_omega` 的栈值取**低 16 位按位回绕**为 `i16`（BAM 语义：反向角速度编码后原始值大于
    /// 32767，例如 65000 == i16 −536），**不钳位、不计数**。判别腿：旧实现把它钳到
    /// `i16::MAX`（最大正转速），此断言必红；合法负角（47332 == −18204）原样写入且不计数。
    #[test]
    fn lz_omega_wraps_bits_not_clamps() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = laser_args(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::from_int(100),
            Fx::from_int(8),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
        let lz = task.stack[0];
        let i = laser_slot(lz);

        let cv0 = w.body.diag.contract_viol;
        // 65000 > 32767：按位回绕为 i16 −536（反向扫射），不计数。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_OMEGA, &[lz, 65000]).is_ok());
        assert_eq!(
            w.body.lasers.omega[i], 65000u16 as i16,
            ">32767 按位回绕为负"
        );
        assert_eq!(w.body.diag.contract_viol, cv0, "按位回绕不计数");

        // 47332 == u16 0xB8C4 == i16 −18204：合法负角速度原样写入，不计数。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_OMEGA, &[lz, 47332]).is_ok());
        assert_eq!(w.body.lasers.omega[i], -18204, "负角速度原样");
        assert_eq!(w.body.diag.contract_viol, cv0, "仍不计数");
    }

    /// 激光几何读口（810–814）：读调用时刻池里的 `ox/oy/angle/start/end`；五个值取互不相同的数，
    /// 读错字段即红。失效句柄押 0、不计数（同 `enemy_x` 与 `lz_alive` 的读族口径）。
    #[test]
    fn lz_readers_return_current_geometry_and_zero_when_stale() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let a = laser_args(
            1,
            Fx::from_int(-30),
            Fx::from_int(70),
            Angle(12345),
            Fx::from_int(420),
            Fx::from_int(8),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
        let lz = task.stack[0];
        let i = laser_slot(lz);
        task.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_LASER_START,
                &[lz, Fx::from_int(32).raw()]
            )
            .is_ok()
        );

        let read = |w: &mut crate::step::World, task: &mut Task, sys: u16, packed: i32| -> i32 {
            task.sp = 0;
            assert!(call(w, &ecl, task, sys, &[packed]).is_ok());
            task.stack[0]
        };
        let cv0 = w.body.diag.contract_viol;
        assert_eq!(
            read(&mut w, &mut task, SYS_LASER_X, lz),
            Fx::from_int(-30).raw()
        );
        assert_eq!(
            read(&mut w, &mut task, SYS_LASER_Y, lz),
            Fx::from_int(70).raw()
        );
        assert_eq!(read(&mut w, &mut task, SYS_LASER_ANGLE, lz), 12345);
        assert_eq!(
            read(&mut w, &mut task, SYS_LASER_NEAR, lz),
            Fx::from_int(32).raw()
        );
        assert_eq!(
            read(&mut w, &mut task, SYS_LASER_FAR, lz),
            Fx::from_int(420).raw()
        );

        // 读的是当前值：改过之后再读，拿到新值。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ROTATE, &[lz, 1000]).is_ok());
        assert_eq!(read(&mut w, &mut task, SYS_LASER_ANGLE, lz), 13345);

        // 失效句柄：五个读口都押 0，不计数。
        w.body.lasers.free_index(i);
        for sys in [
            SYS_LASER_X,
            SYS_LASER_Y,
            SYS_LASER_ANGLE,
            SYS_LASER_NEAR,
            SYS_LASER_FAR,
        ] {
            assert_eq!(read(&mut w, &mut task, sys, lz), 0, "syscall {sys}");
        }
        assert_eq!(read(&mut w, &mut task, SYS_LASER_X, -1), 0);
        assert_eq!(
            w.body.diag.contract_viol, cv0,
            "读口一律不计数（含失效句柄）"
        );
    }

    /// Review Focus 3：激光 A 回收、新激光 B 复用同槽后，拿 A 的旧句柄调 `lz_rotate`
    /// 不得改到 B；计一次违约；`lz_alive(A) == 0`、`lz_alive(B) == 1`。
    #[test]
    fn stale_laser_handle_does_not_touch_reused_slot() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let a = laser_args(
            1,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::from_int(100),
            Fx::from_int(16),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
        let packed_a = task.stack[0];
        let ia = laser_slot(packed_a);
        let gen_a = w.body.lasers.generation[ia];

        // 模拟相位 5 回收 A，再用 B 复用同槽（最低空位优先）。
        w.body.lasers.free_index(ia);
        task.sp = 0;
        let b = laser_args(
            2,
            Fx::ZERO,
            Fx::ZERO,
            Angle(1000),
            Fx::from_int(200),
            Fx::from_int(8),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &b).is_ok());
        let packed_b = task.stack[0];
        let ib = laser_slot(packed_b);
        assert_eq!(ib, ia, "最低空位复用同槽");
        assert_ne!(w.body.lasers.generation[ib], gen_a, "复用槽代际必须前进");
        let angle_b = w.body.lasers.angle[ib];

        let cv0 = w.body.diag.contract_viol;
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ROTATE, &[packed_a, 1234]).is_ok());
        assert_eq!(w.body.lasers.angle[ib], angle_b, "旧句柄不得改到复用槽的 B");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "失效句柄计一次");

        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ALIVE, &[packed_a]).is_ok());
        assert_eq!(task.stack[0], 0, "A 已回收");
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ALIVE, &[packed_b]).is_ok());
        assert_eq!(task.stack[0], 1, "B 存活");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "lz_alive 只读不计数");
    }

    /// `lz_anchor` 挂上活敌后立即吸附、相位 5 逐帧跟随；`lz_anchor(lz, -1, …)` 解除挂靠。
    #[test]
    fn lz_anchor_follows_enemy_then_detaches_with_neg1() {
        use crate::input::InputFrame;
        let (mut w, ecl) = fresh();
        let e = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
        let ei = e.index as usize;
        w.body.enemies.vx[ei] = Fx::from_int(2);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: e.index,
            owner_gen: e.generation,
            ..Task::default()
        };
        let a = laser_args(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::from_int(500),
            Fx::from_int(16),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
        let lz = task.stack[0];
        let i = laser_slot(lz);

        // brief 的原句：`lz_anchor($self_enemy, 0, 8)`——先经 `$self_enemy` 拿打包敌号。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ENEMY, &[]).is_ok());
        let self_enemy = task.stack[0];
        task.sp = 0;
        let anchor = [lz, self_enemy, Fx::ZERO.raw(), Fx::from_int(8).raw()];
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &anchor).is_ok());
        assert_eq!(w.body.lasers.anchor_idx[i], e.index, "挂上敌下标");
        assert_eq!(w.body.lasers.ox[i], Fx::ZERO, "出生帧立即吸附 x");
        assert_eq!(w.body.lasers.oy[i], Fx::from_int(108), "出生帧立即吸附 y+8");

        // 相位 5 一帧：敌人 x += 2，激光跟到 2。
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(2));
        assert_eq!(w.body.lasers.oy[i], Fx::from_int(108));

        // -1 解除：锚态清空、原点留在原地。
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &[lz, -1, 0, 0]).is_ok());
        assert_eq!(
            w.body.lasers.anchor_idx[i],
            crate::lasers::ANCHOR_NONE,
            "-1 解除挂靠"
        );
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(2), "解除不动原点");
    }

    /// 控制方裁定 ②：失效敌号（非 -1，已死/代际不符）→ 不挂靠 + 计一次违约。
    #[test]
    fn lz_anchor_stale_enemy_is_noop_and_counted() {
        let (mut w, ecl) = fresh();
        let e = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
        let mut task = Task::default();
        let a = laser_args(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::from_int(500),
            Fx::from_int(16),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
        let lz = task.stack[0];
        let i = laser_slot(lz);

        let live = crate::enemy::pack_handle(e);
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &[lz, live, 0, 0]).is_ok());
        assert_eq!(w.body.lasers.anchor_idx[i], e.index);
        assert_eq!(w.body.lasers.anchor_gen[i], e.generation);

        // 杀敌回收槽 → 旧敌号失效（非 -1）。
        w.body.enemies.free(e);
        let cv0 = w.body.diag.contract_viol;
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &[lz, live, 0, 0]).is_ok());
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "失效敌号计一次");
        assert_eq!(
            w.body.lasers.anchor_idx[i], e.index,
            "失败调用是 no-op：不改锚态"
        );
    }

    /// 跨任务陷阱（阶段终审 I-3）：ECL 敌号只带代际低 15 位，而相位 5 的挂靠比较用**完整
    /// u16 代际**。`lz_anchor` 必须经 `resolve_enemy_handle` 取下标、再用池里的完整代际重建
    /// 句柄——用低 15 位复刻时，代际越过 0x7FFF 后挂靠会立刻静默脱钩。
    #[test]
    fn lz_anchor_survives_enemy_generation_above_15_bits() {
        use crate::input::InputFrame;
        let (mut w, ecl) = fresh();
        let e = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
        let ei = e.index as usize;
        w.body.enemies.generation[ei] = 0xF00D;
        w.body.enemies.vx[ei] = Fx::from_int(2);
        let mut task = Task::default();
        let a = laser_args(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::from_int(500),
            Fx::from_int(16),
            0,
            9999,
            0,
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
        let lz = task.stack[0];
        let i = laser_slot(lz);

        let packed = crate::enemy::pack_handle(EnemyHandle {
            index: e.index,
            generation: 0xF00D,
        });
        assert_eq!(packed >> 16, 0x700D, "前提：句柄只带低 15 位");
        task.sp = 0;
        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_LASER_ANCHOR,
                &[lz, packed, 0, 0]
            )
            .is_ok()
        );
        assert_eq!(
            w.body.lasers.anchor_gen[i], 0xF00D,
            "必须存池里的完整 u16 代际"
        );

        // 若用低 15 位重建，anchor_gen 会落 0x700D，相位 5 代际不符 → 立刻脱钩。
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.lasers.anchor_idx[i], e.index, "代际相符：仍挂靠");
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(2), "跟随敌人本帧位移");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(4));
    }
}
