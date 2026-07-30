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
| 4 | `CALL` | canonical `SubId` | — | 目标须为 `CallOnly`（否则 Fault(0)）；压返回地址进调用栈（深 8），跳入 sub |
| 5 | `RET` | — | — | 弹返回地址跳回 |
| 10 | `PUSHI` | 立即数 | 压 1 | |
| 11 | `PUSHL` | 槽号 <64 | 压 1 | 读 locals |
| 12 | `POPL` | 槽号 <64 | 弹 1 | 写 locals |
| 13 | `DUP` | — | 压 1 | |
| 14 | `POP` | — | 弹 1 | |
| 20-25 | `ADD SUB MUL DIV MOD NEG` | — | 弹 2 压 1（NEG 弹 1 压 1） | i32 **wrapping** 语义；DIV/MOD 除零 Fault(4)、`MIN/-1` 回绕 |
| 30-33 | `MULF DIVF SINB COSB` | — | 同上/弹 1 压 1 | Q16.16（i64 中间量）；SINB/COSB 取栈顶低 16 位 BAM 查表 |
| 40-45 | `EQ NE LT LE GT GE` | — | 弹 2 压 1 | 压 0/1 |
| 50 | `SPAWN` | canonical `SubId`, argc | 弹 argc，压 1（任务号或 -1） | 目标须为 `Async` 且参数数目精确匹配；owner 继承、parent=自己、**次帧首跑**；池满压 -1 + `pool_full[POOL_TASK]` |
| 51 | `KILL_SELF` | — | — | 即刻完成语义 |
| 52 | `KILL_CHILDREN` | — | — | 升序杀**直系**子任务（不递归） |
| 60 | `SYS` | syscall 号 | 按号 | 一切副作用唯一通道（白名单） |

### `mark` 降低（纯前端语法糖，无新 op；整局流程刀 spec §2）

`mark(id)` / `mark(id) { 补偿块 }` 不新增 opcode——降低为既有 `JMP`/`PUSHI`/`SYS` 的固定
序列：`JMP after; landing: <自动补偿 push_i/sys 序列><作者块>; after:`；正常流一步 `JMP`
跨过整段垫片，中段启动直接把根任务 `pc` 摆到 `landing`（= `EclImage::resolve_mark(id)` 的
返回值）。标记表（`(id, ip)` 严格升序对）随镜像一起产出，存在 `EclImage` 内部私有字段
（`try_from_parts` 校验 id 正整数/严格升序/落点在 `code` 边界内），不占字节码正文；
`resolve_mark` 对它做二分查找。表层参考见 [`ecl-lang.md`](ecl-lang.md)"mark（中段启动
标记）"节。

### SubKind 检查（Root / Async / CallOnly）

`EclImage` 中每个 sub 有且仅有以下三类之一：

- **`Root`**：唯一的 `sub main()`。只能通过引擎 API `start_main` / `start_main_with_owner` 启动。
  运行时 `CALL` 或 `SPAWN` 指向 Root 会触发 Fault(0)（坏操作数值）。
- **`Async`**：`async sub` 声明。注册为 public named entry，可通过 `image.resolve_entry(name)` 按名解析，
  然后经 `world.spawn_entry` / `world.spawn_entry_named` 或运行期 `SPAWN` 指令启动。
  `CALL` 指向 Async sub 触发 Fault(0)。
- **`CallOnly`**：普通 `sub` 声明。只能被 `CALL` 指令（来自其他 sub 的同步调用）进入。
  不在 `EclImage` 的 entry 表中；`SPAWN` 指向 CallOnly sub 触发 Fault(0)。
  名称只存在于调试符号侧载（`DebugInfo::Full` 模式）。

### 外部绑定错误边界（binding error boundary）

引擎层（`stg_core::ecl::binding`）提供三个安全入口替代裸 `spawn_task`：

1. **`world.start_main(&image)`** / **`world.start_main_with_owner(&image, owner)`** —— 启动 Root
   (main)，单次生命周期（`MainAlreadyStarted` 保护）。
2. **`world.spawn_entry(entry, args, owner)`** —— 以已解析的 `ResolvedEntry` 和 `&[i32]` 参数启动，
   快路径（仅校验参数数量）。
3. **`world.spawn_entry_named(&image, name, &[EclArg], owner)`** —— 按名解析 + 类型化参数校验（数量+类型）。

入口在失败时写入 `diag.contract_viol` / `pool_full` / `last_status` 并返回
`TaskStartError` 枚举（`NoRoot` / `MainAlreadyStarted` / `UnknownEntry` /
`RootRequiresStartMain` / `InvalidEntryId` / `WrongArgCount` / `WrongArgType` /
`InvalidOwner` / `PoolFull`），调用方据此决定重试/回退/报错。**不 Fault、不 panic**
（P4-b 确定性安全结果）。

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
| 11 | `spell_timer`（符卡机构，见下方"符卡计器"） | — | owner 绑定槽 `frames_left`；owner 非敌或无绑定槽 → **-1**（`wait_spell` 糖的判据） |
| 12 | `enemy_hp`（A5 补遗） | handle | 活敌 `hp`；死/悬垂/越界句柄 → **-1**（P4-b，不比对 generation，不 Fault——stage 编排等 boss 死的轮询原语） |
| 20 | `create_bullet` | appearance,x,y,speed,angle,xform_off,xform_cnt,task_sub | 弹句柄或 -1 |
| 21 | `create_bullets_batch` | appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step | 实发数 |
| 22 | `spawn_enemy` | x,y,hp,drop_table,score,sprite,task_sub | 敌句柄或 -1（A5 乙案：`task_sub` 同 20 号 `create_bullet` 同款 canonical `SubId`/-1=none；`task_sub>=0` 时必须指向零参数 `Async` sub，绑定层派子任务，owner=新敌） |
| 23 | `drop_item` | x,y,item_type | 道具句柄或 -1 |
| 24 | `move_enemy_to` | **dur,x,y,easing** | —（owner 须为敌，否则 Fault；参数序以 syscall.rs 为准，勿凭直觉写 x,y 在前） |
| 25 | `boss_set` | slot,hp_ratio,spell_id,timer,phase_left,active | —（enemy 字段写 NULL，见 boss_ui 契约） |
| 26 | `pulse_signal` | ch | — |
| 27 | `emit_req` | id a0 a1 a2 a3 a4 a5 | — |
| 28 | `spell_begin`（符卡机构，见下方"符卡计器"） | slot,spell_id,pattern_sub,time_limit,bonus0,flags,hp_threshold | —（owner 须为敌，否则 Fault） |
| 29 | `spell_end`（符卡机构，见下方"符卡计器"） | — | —（owner 绑定槽走 HP 路径结算；无绑定槽 → no-op，重复调用安全） |
| 30-38 | 弹 setter 族 | 按 motion.rs 九连 | —（owner 须为弹，否则 Fault） |
| 40 | `aim_player_angle` | — | 自 owner 位置瞄 P0 的 BAM 角 |
| 50 | `add_score`（整局流程刀，见 consts.rs 5x 族） | delta | —（自机 0 记分；`delta` 允许负值扣分，结果**饱和钳** `[0, u64::MAX]`——扣穿停在 0、加满停在上限，不回绕；不做参数收窄，不 Fault，owner 类别无限制） |
| 51 | `bgm` | id | —（写表现锚点 `bgm_id` + 发 `REQ_BGM`；`id` 收窄 `0..=65535`，越界 → no-op + `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不 Fault**（P4-b），owner 类别无限制） |
| 52 | `bg` | id | —（同上，写 `bg_id` + 发 `REQ_BG`；同一收窄/no-op 口径） |
| 53 | `bg_phase` | n | —（写 `bg_phase`，同时把 `bg_phase_frame` 盖为当前帧，再发 `REQ_BG_PHASE`；`n` 同上收窄/no-op 口径） |
| 54 | `clear_bullets`（B19） | — | —（**0 参**；调 `create_field(field::fullscreen_clear_field())` 铺一个覆盖全场、`life=1`、`FIELD_CLEAR_BULLETS` 的作用区，当帧相位 6 生效——消弹转星星与 `EVT_FIELD_CLEARED` 都是消弹区机制白送的，syscall 层零新逻辑；不做参数收窄；**P4-a**：field 池（cap 16）满 → 走 `create_field` 自身降级（NULL + `diag.pool_full[POOL_FIELD]` +1），**不 Fault**；owner 类别无限制） |
| 55 | `add_lives`（B20） | delta | —（自机 0；`delta` 允许负，`saturating_add` 后**双边钳** `[0, u8::MAX]`——扣穿停 0、加满停 255，不回绕不 panic；**钳位是正常语义**，不计 `contract_viol`、不 Fault（同 `add_score` 口径）；不做参数收窄，owner 类别无限制） |
| 56 | `add_bombs`（B20） | delta | —（同 55，写 `bombs`，钳 `[0, u8::MAX]`） |
| 57 | `add_power`（B20） | delta | —（同 55，写 `power`，但上钳是 `items::POWER_MAX`=**400**（显示 4.00）**而非 `u16::MAX`**——越过它 `power_tier` 档位索引 OOB） |
| 58 | `drop_clear`（敌死效果刀） | — | —（**0 参**；把 self 敌的 `drop_count[..]` 五槽清零。**owner 须为敌**，否则 Fault(0)（同 `move_enemy_to`）；悬垂 owner 句柄 → no-op + `contract_viol` + `STALE_HANDLE`，不 Fault。参照 ZUN `dropClear`(506)） |
| 59 | `drop_add`（敌死效果刀） | type,n | —（逆序弹栈 `n, type`；给 self 敌的待掉落计数**增量**加 `n` 颗 `type`，**只增不减**（人类裁定，清空用 58）。`type` 收窄 `[0, items::ITEM_TYPE_COUNT)`，越界 → no-op + `contract_viol` + `BAD_ARGS`，**不 Fault**（P4-b）；`n` **先钳** `[0, u8::MAX]`（负 n 视同 0）**再 `saturating_add`** 到计数上——两步都要，只钳不饱和会在近 255 时 debug panic，只饱和不钳会让负 n `as u8` 回绕。owner 须为敌，否则 Fault(0)。参照 ZUN `dropExtra`(507)） |
| 60 | `drop_items`（敌死效果刀） | — | —（**0 参**；立刻把 self 敌的待掉落计数撒出去（按类型编号升序逐颗 `spawn_drop`，**消耗世界 RNG**）。**吐完不清空计数**（人类裁定，照 ZUN 字面）——故 `drop_items(); die();` 掉**双份**，作者自负；**不加分、不发 `EVT_ENEMY_DIED`、不发 `REQ_ENEMY_DEATH`、不标 `ENEMY_DYING`**；对已 dying 的敌照撒不误（无幂等门禁，与 61 不同）。池满走 `spawn_drop` 自身的 P4-a 逐颗降级。owner 须为敌，否则 Fault(0)。参照 ZUN `dropItems`(509)） |
| 61 | `die`（敌死效果刀） | — | —（**0 参**；对 self 敌跑**完整死亡效果**（`world::settle::kill_enemy`）：`hp = hp.min(0)` → 标 `ENEMY_DYING` → 撒掉落 → `enemies.score` 记进自机 0 → `EVT_ENEMY_DIED` → `REQ_ENEMY_DEATH`。**幂等**：已 dying → 直接返回。**只标记不回收**，相位 9 cleanup 才收尸——当帧体碰仍成立。表层 `die()` 由 codegen 降低成 **`SYS 61` + `OP_KILL_SELF` 两条指令**，故调用它的任务立即终止（人类裁定），后续语句不执行；**手写字节码只发 `SYS 61` 不会终止任务**。owner 须为敌，否则 Fault(0)。参照 ZUN `die`(561)——ZUN 那条还经 `setDeath`(556) 间接一层，留给 `death_script` 通电那一刀） |

`create_bullet` 走**丙方案**：`(xform_off, xform_cnt)` 指向本任务 locals 内打包槽
（每槽 3 字：`word0=(wait<<16)|(op<<8)`、`word1/2=args`，≤16 槽）；`xform_cnt=0` 哑弹；
`task_sub >= 0` 时其值是 canonical `SubId`，且必须指向零参数 `Async` sub；绑定层再派子任务
（owner=新弹）；appearance 查 `WorldTables.appearances`
定默认 radius/sprite。

- **`emit_req`（27）**：通道 B 渲染请求（`docs/ecl-lang.md`"渲染请求"节）。id 收窄 P4-b：
  栈值超出 `0..=65535` → no-op + `contract_viol` + `BAD_ARGS`，不 Fault；缓冲满走 D12
  （丢弃 + `TRUNCATED` + `diag.reqs_dropped`）。无 owner 类别限制（STAGE 任务可发）。

## 符卡计器（syscall 11/28/29 + `wait_spell` 糖；spec 2026-07-24）

记账（计时/衰减/超时/破卡/`boss_ui` 喂送）全归引擎 `SpellState` 机构（settle 相位符卡趟），
三条 syscall 是脚本唯一的操作面；表层参考见 [`ecl-lang.md`](ecl-lang.md)"符卡"节。

- **`spell_begin`（28）**：参数**逆序弹栈**（`threshold, flags, bonus0, time_limit,
  pattern_ref, spell_id, slot`）；owner 须为 `ENEMY`，否则 Fault(0)（同 `move_enemy_to`
  误用策略）。`pattern_ref`（`SubRef`，负值=none）**先查后建**：越界/非零参 `Async` 号 →
  Fault(0)（零副作用，同 `create_bullet`/`SPAWN` 坏号口径）；查通过后才调用世界层
  `spell_begin_internal`，其 P4-b 拒收条件（no-op + `contract_viol` 计数，不 Fault）：
  `threshold<0`、`bonus0<0`、`time_limit<=0`、槽越界、槽已 `active`、owner 已绑定另一槽。
  成功：定格 bonus 衰减参数（地板/速率）+ 记 `hp_start` + `pattern_ref` 非 none 时照
  `create_bullet` 的 task-spawn 样板 spawn 模式任务（owner=本敌，`spell_bound=slot+1`
  绑定该卡槽，随卡生死见下）+ 发 `EVT_SPELL_DECLARED` 事件 + `REQ_SPELL_DECLARE` 请求。
- **`spell_end`（29）**：无参，owner 须为 `ENEMY`（Fault(0) 同上）。owner 有绑定槽 → 走
  HP 路径结算（资格在 → CAPTURED 付 `bonus_now`；资格失 → FAILED）；无绑定槽 → no-op
  （不计数，重复调用安全——脚本可以无条件调它当"确保收尾"）。
- **`spell_timer`（11）**：无参，读族。owner 非 `ENEMY` 或无绑定槽 → **押 -1**（同
  `self_hp`/`self_hp_max` 误用降级口径——不 Fault，方便脚本用 `>= 0` 判活）；有绑定槽 →
  该槽 `frames_left`。这个 `-1` 判据正是 `wait_spell()` 糖的展开条件。

**`wait_spell()` 语法糖**（纯编译器前端，零 VM/字节码改动）：`lang::parse` 直接把
`wait_spell();` 展开成等价的 `Stmt::While` 子树，等同于脚本作者手写
`while spell_timer() >= 0 { wait(1); }`——**没有专属 op、没有专属 syscall**，codegen 拿到
展开后的 AST 和手写的 while 一视同仁，产出逐字节相同的 `EclImage`（判别测试见
`stg-ecl-compiler::lang::codegen::tests`）。字节码层看到的永远只是普通的 `JZ`/`SYS 11`/
`WAIT` 组合，本表其余各条纪律（返回值必须消费、死循环需要 `WAIT` 等）照常适用。

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
- **`KILL_CHILDREN` 按父槽号匹配，父死即断亲**（follow-ups C12⑤ 已修复，2026-07-20）：
  `TaskPool::kill` 释放一个槽时会顺手把所有指向它的存活子任务的 `parent` 清零——孤儿从
  父死那一刻起就是永久无父的独立任务，槽复用后新占用者的 `KILL_CHILDREN` 不会误杀前任
  的孤儿（本池无逐槽 generation，这是"父死立即断亲"代替代际戳的等价修法）。
- 难度（rank）= `globals` 约定槽（场景开局 `set_var` 写入），脚本 `get_var` 后自决——该槽
  （`GVAR_RANK=0`）落在**系统段**，脚本自己 `set_var(0, ..)` 会被 no-op 挡下（见"globals 段
  纪律"），想改 rank 得靠 game 层世界 API，不是脚本自己改自己的难度。
- **`self_age` 是任务龄不是敌龄**：`SYS 9`（M1.5）量 `frame - task.born_frame`，次帧首跑时
  已经是 `1`（不是 `0`，出生当帧被 born_frame 门禁跳过、根本不执行）；对 `SPAWN` 出的子任务
  同理——子任务自己的 `born_frame` 是它的出生帧，不是父任务或 owner 实体的出生帧。
