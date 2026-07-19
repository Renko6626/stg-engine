# ECL op / syscall 速查表（M1）

> **字节码层参考**——脚本作者请看 [`ecl-lang.md`](ecl-lang.md)（表层语言手册，M1.9 起）；
> 本表服务于 VM/编译器/绑定层开发与调试。
>
> **这是什么**：ECL VM 底层参考——opcode、syscall 号、fault 码、字节码级须知。
> **权威来源**（冲突时以它们为准）：`crates/stg-core/src/ecl/{ops,syscall,vm}.rs` 常量
> （**编号即契约**，冻结纪律同 D4：增删改 = 过评审 + bump engine_ver）·
> spec `docs/superpowers/specs/2026-07-18-m1-ecl-vm-design.md`。
> DSL（stg-ecl-compiler builder）是**临时形态**——表层语言另立刀时本表仍是字节码层契约。

## 指令编码

字流：1 头字（opcode 在低 8 位，余位留白）+ N 操作数字（N = `ARITY[op]`）。
立即数内联；动态值走求值栈（32 字深）。

## op 表（十位=族号留空隙）

| ID | 名 | 操作数字 | 栈效果 | 语义 |
|---|---|---|---|---|
| 0 | `END` | — | — | 任务正常完成（杀自己，无事件） |
| 1 | `WAIT` | — | 弹 1（帧数） | 写 `wait` 并让出；帧首 `wait>0` 递减跳过 |
| 2 | `JMP` | 目标 pc | — | 无条件跳 |
| 3 | `JZ` | 目标 pc | 弹 1（条件） | 条件==0 跳，否则顺序 |
| 4 | `CALL` | 目标 pc | — | 压返回地址进调用栈（深 8），跳入 sub |
| 5 | `RET` | — | — | 弹返回地址跳回 |
| 10 | `PUSHI` | 立即数 | 压 1 | |
| 11 | `PUSHL` | 槽号 <64 | 压 1 | 读 locals |
| 12 | `POPL` | 槽号 <64 | 弹 1 | 写 locals |
| 13 | `DUP` | — | 压 1 | |
| 14 | `POP` | — | 弹 1 | |
| 20-25 | `ADD SUB MUL DIV MOD NEG` | — | 弹 2 压 1（NEG 弹 1 压 1） | i32 **wrapping** 语义；DIV/MOD 除零 Fault(4)、`MIN/-1` 回绕 |
| 30-33 | `MULF DIVF SINB COSB` | — | 同上/弹 1 压 1 | Q16.16（i64 中间量）；SINB/COSB 取栈顶低 16 位 BAM 查表 |
| 40-45 | `EQ NE LT LE GT GE` | — | 弹 2 压 1 | 压 0/1 |
| 50 | `SPAWN` | script id | 压 1（任务号或 -1） | 派子任务：owner 继承、parent=自己、**次帧首跑**；池满压 -1 + `pool_full[POOL_TASK]` |
| 51 | `KILL_SELF` | — | — | 即刻完成语义 |
| 52 | `KILL_CHILDREN` | — | — | 升序杀**直系**子任务（不递归） |
| 60 | `SYS` | syscall 号 | 按号 | 一切副作用唯一通道（白名单） |

## syscall 号表 v1（参数正序压栈、派发逆序弹栈；返回值压栈**须消费或 POP**）

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 0 | `frame` | — | 帧号 |
| 1/2 | `player_x/y` | — | P0 坐标（Fx raw） |
| 3/4 | `self_x/y` | — | owner 实体坐标（STAGE 读 0） |
| 5 | `self_hp` | — | owner 敌 hp（非敌读 0） |
| 6 | `rand_range` | n | [0,n) 消耗世界 RNG；n≤0 压 0 不消耗 |
| 7/8 | `get_var` / `set_var` | slot / slot,val | 值 / —（`get_var` 两段皆无限制；`set_var` 写**系统段**
  `slot<16` → no-op + `diag.contract_viol` 计数，**不 Fault**——见下方"globals 段纪律"及
  `world::GLOBALS_SYS_SEGMENT` 文档） |
| 9 | `self_age`（M1.5） | — | 任务龄（帧）= `frame - task.born_frame`（wrapping）。**语义故意
  偏离 ZUN**：ZUN `-9988` 是"敌出生以来帧数"（只对敌有意义）；我们量的是**任务**的龄——零新
  状态（复用既有 `Task.born_frame`），对全部 owner 种类（含 STAGE）均有意义。次帧首跑时
  `age==1`（不是 0），见"作者须知" |
| 10 | `self_hp_max`（M1.5） | — | owner 敌 `hp_max`（非敌读 0，同 `self_hp` 误用策略） |
| 20 | `create_bullet` | appearance,x,y,speed,angle,xform_off,xform_cnt,task_script | 弹句柄或 -1 |
| 21 | `create_bullets_batch` | appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step | 实发数 |
| 22 | `spawn_enemy` | x,y,hp,drop_table,score | 敌句柄或 -1 |
| 23 | `drop_item` | x,y,item_type | 道具句柄或 -1 |
| 24 | `move_enemy_to` | **dur,x,y,easing** | —（owner 须为敌，否则 Fault；参数序以 syscall.rs 为准，勿凭直觉写 x,y 在前） |
| 25 | `boss_set` | slot,hp_ratio,spell_id,timer,phase_left,active | —（enemy 字段写 NULL，见 boss_ui 契约） |
| 26 | `pulse_signal` | ch | — |
| 30-38 | 弹 setter 族 | 按 motion.rs 九连 | —（owner 须为弹，否则 Fault） |
| 40 | `aim_player_angle` | — | 自 owner 位置瞄 P0 的 BAM 角 |

`create_bullet` 走**丙方案**：`(xform_off, xform_cnt)` 指向本任务 locals 内打包槽
（每槽 3 字：`word0=(wait<<16)|(op<<8)`、`word1/2=args`，≤16 槽）；`xform_cnt=0` 哑弹；
`task_script >= 0` 时绑定层再派子任务（owner=新弹）；appearance 查 `WorldTables.appearances`
定默认 radius/sprite。

## globals 段纪律（甲案，M1.5）

`globals`（`GLOBALS_CAP=1024` 槽）拆两段，边界 `world::GLOBALS_SYS_SEGMENT=16`：

- **系统段** `[0, 16)`：只准 **game 层经世界 API** `WorldBody::set_var` 写（场景搭建代码，如
  `stg-harness` 建场时 `world.body.set_var(GVAR_RANK, ..)`）；**脚本经 `SYS 8 set_var` 写这段
  一律 no-op**——真槽值不变 + `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不 Fault**
  （P4-b 脚本作者违约的确定性安全结果，调度层不杀任务，脚本继续往下执行）。已命名槽：
  `GVAR_RANK=0`（难度，见下方"作者须知"）。
- **自由段** `[16, 1024)`：脚本读写皆无限制。
- `SYS 7 get_var`（脚本读）**两段皆不受限**——只有脚本**写**系统段被挡，读不受影响。

## Fault 码（确定性报错：杀任务 + `EVT_TASK_FAULT{a_index=任务号, data=[码, script]}` + `diag.task_faults`）

| 码 | 含义 |
|---|---|
| 0 | 未知 op / **坏操作数值**（locals 槽号越界、坏 syscall 号、坏 owner 类别等复用此码） |
| 1 | pc/跳转目标/操作数字越界 |
| 2 | 求值栈上溢或下溢（含 syscall 参数不足） |
| 3 | 指令预算耗尽（任务 1024/帧 或 全局 65536/帧，升序消耗先到先杀） |
| 4 | 除零 |
| 5 | 调用深度超限 / 空栈 RET |

## 作者须知（踩过/会踩的坑）

- **locals 任务全局共享**：sub 调用不开新窗口——递归共享变量；DSL `repeat` 计数器占用
  locals 高端（63 向下递减），用户槽从 0 向上，别相撞。
- **返回值必须消费**：每个有返回的 syscall 都压栈——不用就 `POP`，否则跨 `loop` 迭代
  累积、**在远离错误点的某个 wait 周期后**以 Fault(2) 爆炸（实战踩过，T4）。
- **次帧首跑**：`SPAWN`/`spawn_task` 的子任务出生当帧不执行——首个效果落在下一帧。
- **owner 死 = 任务静默死**（无事件无计数，常态非错误）；显式拆树用 `KILL_CHILDREN`。
- **死循环必须带 `WAIT`**：纯循环烧满 1024 条/帧即被杀（响亮地死，Fault 3）。
- **`WAIT` 取栈顶低 16 位**：`wait(-1)` = 等 65535 帧、`wait(65536)` = 等 0 帧（截断语义，
  测试钉死）——帧数走大数请分段。
- **`KILL_CHILDREN` 按父槽号不带代际**：父任务死亡后其槽被无关新任务复用时，新任务的
  `KILL_CHILDREN` 会杀到前任占用者的孤儿子任务（确定性、detached 语义下孤儿本就随时可死，
  但语义上是跨族误杀——代际戳修法记 follow-ups C12⑤，介意就别依赖孤儿存活）。
- 难度（rank）= `globals` 约定槽（场景开局 `set_var` 写入），脚本 `get_var` 后自决——该槽
  （`GVAR_RANK=0`）落在**系统段**，脚本自己 `set_var(0, ..)` 会被 no-op 挡下（见"globals 段
  纪律"），想改 rank 得靠 game 层世界 API，不是脚本自己改自己的难度。
- **`self_age` 是任务龄不是敌龄**：`SYS 9`（M1.5）量 `frame - task.born_frame`，次帧首跑时
  已经是 `1`（不是 `0`，出生当帧被 born_frame 门禁跳过、根本不执行）；对 `SPAWN` 出的子任务
  同理——子任务自己的 `born_frame` 是它的出生帧，不是父任务或 owner 实体的出生帧。
