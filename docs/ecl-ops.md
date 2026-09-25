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
| 1 | `WAIT` | — | 弹 1（帧数 n） | **等 n 帧**（周期 == n）：写 `wait = n−1` 并让出；帧首 `wait>0` 递减跳过。**n==0 不让出、同帧继续**（真 no-op） |
| 2 | `JMP` | 目标 pc | — | 无条件跳 |
| 3 | `JZ` | 目标 pc | 弹 1（条件） | 条件==0 跳，否则顺序 |
| 4 | `CALL` | canonical `SubId` | — | 目标须为 `CallOnly`（否则**加载时拒绝**，见下方"加载时代码校验"）；压返回地址进调用栈（深 8），跳入 sub |
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
  `CALL` 或 `SPAWN` 指向 Root **在加载时就被拒**（`ImageBuildError::BadCode`，`reason` 分别是
  `CallTarget`/`SpawnTarget`；引擎第二刀 §4 起——此前是运行时 Fault(0)，见下方"加载时代码
  校验"）。
- **`Async`**：`async sub` 声明。注册为 public named entry，可通过 `image.resolve_entry(name)` 按名解析，
  然后经 `world.spawn_entry` / `world.spawn_entry_named` 或运行期 `SPAWN` 指令启动。
  `CALL` 指向 Async sub **在加载时就被拒**（同上）。
- **`CallOnly`**：普通 `sub` 声明。只能被 `CALL` 指令（来自其他 sub 的同步调用）进入。
  不在 `EclImage` 的 entry 表中；`SPAWN` 指向 CallOnly sub **在加载时就被拒**（同上）。
  名称只存在于调试符号侧载（`DebugInfo::Full` 模式）。

### 加载时代码校验（引擎第二刀 §4，2026-09-24）

`EclImage::try_from_parts`（`image.rs`）末尾的 `validate_code` 把下面这些**静态可判**的
问题从"运行时才第一次踩到"提前到"镜像构造时就拒绝并返回 `Err`"：坏 op（含头字保留位
非零）、操作数越出 `code` 末尾、跳转/CALL/SPAWN 目标落在非法位置或非指令边界、局部
下标越界、syscall 号不在白名单。编译器侧（`ImageBuilder::build`）把它当编译错误直接
抛给脚本作者，落地脚本这类错误现在会在 `stg-harness check` 那一步就报出来，不必真的
跑起来才能撞见。

错误类型是 `ImageBuildError::BadCode { pc: u32, reason: BadCodeReason }`：`pc` 是问题
所在指令自身的字索引（sub `code_entry`/mark `ip` 未落在指令边界上的情形例外——那时
`pc` 就是那个非法落点本身）；`reason` 见 `BadCodeReason`（`UnknownOp`/`ReservedBits`/
`TruncatedOperand`/`JumpTarget`/`NotInstructionBoundary`/`CallTarget`/`SpawnTarget`/
`SpawnArgc`/`LocalIndex`/`Syscall`）。

**运行时仍保留、不会被校验器取代的检查**（见下方 Fault 表）：取指越界与操作数越界
（`FAULT_PC_OOB`）——`World::load_bytes` 会原样恢复存档里的 task pc，镜像哈希只证明
"表一致"，证不了"pc 落在这份脚本的合法指令边界上"，跨镜像存档或手改过的存档能把
`task.pc` 带到任意位置，这条检查是 P4（调用方违约→确定性安全结果，不 panic）的最后
一道闸；此外栈深/下溢、调用深度、除零、SPAWN 的 `sp < argc`、syscall 内部依赖运行时
栈值/owner kind 的动态检查，都继续在运行时判。

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

## syscall 号表（百分区制；参数正序压栈、派发逆序弹栈；返回值压栈**须消费或 POP**）

> **百分区制（2026-07-31 重排）**：百位 = 族号。**新号落族内、永不乱序追加**；族满了走评审
> 开新族，不许溢出到隔壁。`4xx`/`5xx`/`6xx` 与 ZUN ECL 同号段有意对齐（他的 4xx = move、
> 5xx = drops、6xx = et\* 弹管理器）。改号 = 冻结面变更 = 过评审 + bump `ENGINE_VER`。
>
> 上一版是**十位**族号制，`0x` 读族（容量 10）实占 13、`6x` shooter（容量 10）实占 15，
> 两次静默溢出之后 77 号起没族可落，最近四刀只好纯自增，读族因此裂成三段。教训不是"纪律松"，
> 是族容量对一张还在长的表本来就不够。新族落 `8xx` 起。
>
> **本刀 74 进 74 出**：只改号，任何 syscall 的语义/参数序/降级口径**一字未改**（spec
> `docs/superpowers/specs/2026-07-31-syscall-renumber-design.md`）。取值以
> `crates/stg-core/src/ecl/syscall.rs` 的 `SYS_*` 常量为唯一权威。

### 0xx —— `$` 引擎变量（12）

与 `parse.rs::resolve_engine_var` 白名单**一一对应**——族内只放"脚本写 `$name` 就能读到的
东西"。owner 派发规则全族同款：ENEMY → 敌池、BULLET → 弹池、其余（含 STAGE）→ 0/零角，
不 Fault、不计 `contract_viol`。

> 号列的三位补零**是排版**（对齐族号），真实操作数不补零——手写字节码写 `SYS 32` 不是 `SYS 032`。
> ⚠️ 本族（000–032）是全表**唯一仍与 op 号域重叠**的一族（op 是 `u8`、现最大 60 = `OP_SYS`），
> 故"裸扫字节流找 opcode"那类假阳性没被重排消掉，见 `lang/mod.rs::opcodes_of` 的注释。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 000 | `frame` | — | 帧号 |
| 010/011 | `player_x/y` | — | **1P**（`players[0]`）坐标（Fx raw）——恒读 1P、不查存活。它是**坐标读、不是瞄准原语**；要瞄准用 120 号 `aim_player`（那条走"最近可瞄自机"口径，F8 统一后两者在 co-op 下会指向不同的人） |
| 020/021 | `self_x/y` | — | owner 实体坐标（STAGE 读 0） |
| 022 | `self_vx`（同刀，引擎变量 `$self_vx`） | — | owner 的 `vx`（`Fx` raw）。**敌→敌池、弹→弹池、其余 owner（含 STAGE）→ 0**（同 020/021 号 `self_x/y` 的既有降级口径）；不 Fault、不计 `contract_viol`。表层是 `$` 引擎变量，号表层与其余 syscall 同一空间（映射见 `builtins::engine_var_info`） |
| 023 | `self_vy`（同刀） | — | owner 的 `vy`（`Fx` raw）——022 号的镜像，逐条同口径 |
| 024 | `self_speed`（同刀） | — | owner 的**速率**（`Fx` raw，作者视图）。与 022/023 号恒同步（双表示：`vx/vy` 是积分真相，`speed/angle` 是作者视图，任一侧被写后另一侧立刻刷新/回填） |
| 025 | `self_angle`（同刀） | — | owner 的**朝向**（BAM raw，作者视图）。⚠️ 表层类型是 **`angle` 不是 `fx`**（能直接喂 420 号 `move_angle` / `fire` 的角度位，与 `fx` 之间无隐式转换）。**近停时冻结**：回填有速度下限（`BACKFILL_MIN_SPEED`），零速下读到的是**最后一次有效朝向**而不是垃圾角——这是刻意的，否则停一帧就把朝向抹掉了 |
| 026 | `self_enemy`（boss 换段刀 2026-09-14，引擎变量 `$self_enemy`） | — | owner 敌的**打包敌号**（编码同 210 号）；owner 非敌 → **-1**。⚠️ 与本族「非敌读 0」**有意不同**：打包敌号 0 是合法值，读 0 分不清。26 不是 op 号，不新增 F6 重叠点 |
| 030 | `self_hp` | — | owner 敌 hp（非敌读 0） |
| 031 | `self_hp_max`（M1.5） | — | owner 敌 `hp_max`（非敌读 0，同 `self_hp` 误用策略） |
| 032 | `self_age`（M1.5） | — | 任务龄（帧）= `frame - task.born_frame`（wrapping）。**语义故意
  偏离 ZUN**：ZUN `-9988` 是"敌出生以来帧数"（只对敌有意义）；我们量的是**任务**的龄——零新
  状态（复用既有 `Task.born_frame`），对全部 owner 种类（含 STAGE）均有意义。次帧首跑时
  `age==1`（不是 0），见"作者须知" |

### 1xx —— 查询（10）

函数形态的读口 + 纯数学。与 `0xx` 的分界是**语法形态**（`$name` vs `f(...)`），**不是**"读/写"。
owner 类别全族无限制。

> ⚠️ `rand_range`(150) 是本族**唯一有副作用者**（推进模拟 PRNG，随快照回滚）——按脚本视角的
> "取一个值"归族，**不是**纯读口。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 100 | `enemy_hp`（A5 补遗；编码见 210 号） | handle | 活敌 `hp`；死/悬垂/越界/**gen 不符** → **-1**（P4-b，不 Fault——stage 编排等 boss 死的轮询原语）。**敌句柄打包刀 2026-07-31**：`handle` 是打包敌号，判据多了一条 generation 比对（`resolve_enemy_handle`），故**槽被另一只敌复用后旧敌号照样返 -1**——打包前这里是条真 bug：boss 死后若有杂兵占了它的槽，`enemy_hp(boss)` 会读到杂兵的血，「等 boss 死」的轮询卡住不退 |
| 101 | `enemy_x`（敌坐标读口刀 2026-07-31） | handle | 按敌号读 **x**（`Fx` raw）。补的是 110 号通电后暴露的断头路：脚本拿得到敌号却读不到坐标，"查最近的敌 → 朝它开火"算不出角度；数据本就在敌池里，缺的只是读口。`handle` 是**打包敌号**（同 100/110 号编码，含 generation ⇒ 槽复用可辨；**敌句柄打包刀 2026-07-31**）。**P4-b 降级**：负句柄 / 越界 / 死槽 / **gen 不符** → **返 `0`**，不 Fault、**不计 `contract_viol`**（纯读族口径，与 100 号共用 `resolve_enemy_handle` 那一份判据 `packed >= 0 && idx < CAP && is_alive(idx) && (generation[idx] & 0x7FFF) == g`，逐字相同、只有降级值不同）；`ENEMY_DYING` 的敌**仍可读**（`is_alive` 是存活位，dying 只是 flag，槽活到相位 9 才回收）。**为什么降级值是 `0` 而不是哨兵**：100 号能用 `-1` 是因为 hp 天然非负，坐标没这个便利——`-1` 是完全合法的 `Fx` raw（≈ −0.0000153），没有哨兵位可用；取 `Fx::ZERO` 与 `self_pos` 对 STAGE owner 的第三分支同款。**代价**：「敌恰在原点」与「号无效」读起来一样，故探活归脚本——先 `enemy_alive(e) == 1` 再读坐标（写进 `docs/ecl-lang.md` 的惯例；103 号落地前是 `enemy_hp(e) != -1`）。⚠️ 探活判据**不是** `enemy_hp(e) >= 0`：overkill 的敌 hp 是真实负值（`settle::kill_enemy` 只 `min(0)`，不抹平负血），`>= 0` 会把"刚被打穿、槽还在"的敌误判成无效。**owner 类别无限制**（关卡任务也该能查）。判别腿把敌放在 **x ≠ y** 的 (30, −70)（放 (5,5) 则两条派发臂写反完全不可辨），外加一条 `.ecl` 源码级 e2e 走完 `nearest_enemy → 探活 → 读坐标 → atan2 → fire` |
| 102 | `enemy_y`（敌坐标读口刀 2026-07-31） | handle | 按敌号读 **y**（`Fx` raw）——101 号的镜像，语义/降级值/存活判据/owner 口径**逐条相同**（两条共用 `sys_enemy_pos(.., want_y)` 一份实现，只差选轴那一位）。详见 101 号 |
| 103 | `enemy_alive`（探活读口刀 2026-07-31；ZUN `555 enmAlive`） | handle | 敌号是否仍指向**它当初那只敌**，押 **1 / 0**（**敌句柄打包刀 2026-07-31** 之前只能判到「槽有效」）。补的是 80/81 落地后留下的残余缝：探活此前只能拿 `enemy_hp(e) != -1` 当探针，而 **`-1` 同时是降级值和一个合法血量**——overkill 的敌 hp 是真实负值（`settle::kill_enemy` 只 `min(0)`，不抹平），血量恰为 −1 的活敌会被旧探针误判成"号无效"；本号是专用口，与血量取值无关。存活判据**逐字同** 100/101/102 号（四条共用 `resolve_enemy_handle`：`packed >= 0 && idx < EnemyPool::CAP && is_alive(idx) && (generation[idx] & 0x7FFF) == g`），只是把判据**本身**押出去而不是拿它选一个值；不 Fault、**不计 `contract_viol`**（纯读族口径），**owner 类别无限制**。⚠️ **语义裁定（人类拍板）：判的是「槽有效」，`ENEMY_DYING` 的敌返 1，不是「还能打」**——读族四条（100/101/102/103）必须用完全相同的三判据，dying 的槽要活到相位 9（坐标仍读得到），四条里单独给一条换判据会让这组口径散掉。配套：110 号 `nearest_enemy` **本身已排除 dying**（候选 = 存活且非 dying），故"从它拿到的号后来变 dying"应当**重查**而不是继续用。判别腿两条——dying 仍返 1（唯一的语义裁定，"排除 dying"是最自然的错法且只有这条测试逮得住）/ 与 `enemy_hp` 的判据一致性（三种无效各断言两者同步，外加"活敌血量恰为 −1"那一格证明新口不是 `enemy_hp` 的花哨包装）；外加一条 `.ecl` 源码级 e2e 与旧探针那条**并存**，正好证明新旧判据在正常路径上等效 |
| 110 | `nearest_enemy`（小清洗刀 2026-07-31） | x,y | 离 `(x,y)` 最近的活敌的**打包敌号**（编码见 210 号）；场上无敌（或全 `ENEMY_DYING`）→ **-1**。世界侧 `world::nearest_enemy` 自 M0-13 就有实现和测试、从没有 syscall 暴露过（死代码），本号只是给它通电，世界侧一行未改：候选 = 存活且非 dying、平方距离 i64 比较、并列取**低索引**（I4）、无距离上限。世界侧返的一直是**完整句柄**，**敌句柄打包刀 2026-07-31** 之前这里只押 `h.index`、把 generation 丢了——本刀不是加机制，是把已有的信息接上；接上之后槽复用可辨，与 100 号 `enemy_hp` 完全同编码（两者本就配对用：本号拿号 → 100 号轮询）。空场返 -1 不计违约（合法世界状态非违约）、不 Fault。**owner 类别无限制**（关卡编排任务也该能查） |
| 120 | `aim_player_angle` | — | 自 owner 位置瞄**目标自机**的 BAM 角。目标 = 从 owner 位置看过去**最近的可瞄自机**（非 ABSENT 非 GAMEOVER）；一个可瞄的都没有 → 回退 `players[0]` 的最后坐标——查询必须产出一个角度，没有"不瞄"这个选项。这是引擎唯一的"瞄谁"口径 `WorldBody::aim_target`，640 号 `sh_aim` 与之同源（F8 统一，2026-09-03）；单人局与旧的"恒 `players[0]`"逐位等同 |
| 130 | `spell_timer`（符卡机构，见下方"符卡计器"） | — | owner 绑定槽 `frames_left`；owner 非敌或无绑定槽 → **-1**（`wait_spell` 糖的判据） |
| 131 | `spell_result`（boss 换段刀 2026-09-14） | slot | 槽 `slot` 最近一次结算的**结束方式**：0 还没结束过 / `SPELL_END_HP`(1) 打到血线（含 boss 死亡、清场杀死） / `SPELL_END_TIMEOUT`(2) 超时 / `SPELL_END_MANUAL`(3) `spell_end`。存于 `WorldBody.spell_last_result`，只由结算写，`spell_begin` 不清。owner 无限制；`slot ∉ [0, MAX_BOSSES)` → 押 0 + `contract_viol` + `BAD_ARGS` |
| 140 | `atan2`（小清洗刀 2026-07-31） | y,x | 方向角（BAM）——**参数序是 `(y, x)`**（同 libm/ZUN 惯例；两位同为 `Fx` raw，对调不判型报错、只静默把角度镜像到另一条对角线上，故 `syscall.rs` 侧钉了**两个**取值的判别腿）。派发臂逆序弹出（先 `x` 后 `y`）。**无 P4 分支**：`math::cordic::atan2`（整数 CORDIC，钉死 16 轮）对任意 `(y, x)` 都有定义，含 `(0,0)`（返 0）——没有"坏参数"这个概念，不计 `contract_viol`、不 Fault。owner 类别无限制。与 120 号 `aim_player_angle` 的关系：那条只能瞄自机（0 参、基点写死 self 位置），本条能瞄任意点/任意敌 |
| 141 | `dist`（小清洗刀 2026-07-31） | dx,dy | 向量 `(dx,dy)` 的**模长**（`Fx` raw）——**不是两点距离**，两点距离由脚本自己减（`dist(bx-ax, by-ay)`）。实现 = `isqrt(len_sq(dx,dy))`：`geom::len_sq` 返 **Q32.32**（`raw²`，不归一化），`isqrt` 开根正好把它变回 **Q16.16**（见 CLAUDE.md「定点乘法规范」）。值域：`len_sq` 恒 ≥0 故 `as u64` 安全；满屏最大约 1.7e15 → 开根 ≈4.1e7，远在 i32 上限内。**收窄回 i32 处饱和不回绕**：上面那句余量论证只覆盖**世界坐标差**这个域，而本条是通用两参 syscall——脚本能直接喂极端 `fx`。`dx = dy = i32::MAX` 时 `len_sq` ≈ 9.22e18（贴着 i64 上限但不溢出），开根 = 3037000498 **超 `i32::MAX`**，裸 `as i32` 会静默回绕成 **-1257966798**（负距离，会往下游传播的无意义值）；故实现是 `isqrt(..).min(i32::MAX as u32) as i32`——确定性降级到最大可表示距离，**钳位是正常语义**，不计 `contract_viol`、不 Fault（同 `add_score`/`add_lives` 族口径）。**无 P4 计数分支**（任意 i32 对都合法）。**为什么不单独暴露 `len_sq`/`isqrt`**（人类裁定）：`len_sq` 返 i64 而脚本值域是 i32 装不下，单独的 `isqrt` 对脚本没有直接用处——`dist` 才是那个有用的组合。owner 类别无限制 |
| 150 | `rand_range` | n | [0,n) 消耗世界 RNG；n≤0 压 0 不消耗 |

### 2xx —— 造物（4）

建实体的四条，全部**押返回值**（句柄或 -1；批量押实发数）——返回值必须消费，见"作者须知"。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 200 | `create_bullet` | appearance,x,y,speed,angle,xform_off,xform_cnt,task_sub | 弹句柄或 -1 |
| 201 | `create_bullets_batch` | appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step | 实发数 |
| 210 | `spawn_enemy` | x,y,hp,drop_table,score,sprite,task_sub,arg0…arg(n−1),argc（**boss 换段刀 2026-09-14 追加**实参与个数；门禁全部先于建敌：argc 越 `[0, LOCALS]` 或栈不够 `argc+7` → Fault(2)；task 为 none 却带参、sub 非 Async 或形参个数 ≠ argc → Fault(0)；实参写进新任务 `locals[0..argc)`，与 `OP_SPAWN` 共用 `TaskPool::write_args`） | **打包敌号**或 -1（**敌句柄打包刀 2026-07-31**：押的不再是裸池 index，而是 `((gen & 0x7FFF) << 16) \| index`。只押 generation 的低 15 位 ⇒ **打包值恒非负**，`-1` 仍是唯一的无效哨兵；代价是 ABA 检测周期从 65536 次同槽复用降到 32768，记在 `docs/follow-ups.md`。脚本侧敌号是**不透明值**——别猜数值、别做算术，**两个敌号相等 ⇒ 同一只敌**。100/101/102/103/110 全族同编码）（A5 乙案：`task_sub` 同 200 号 `create_bullet` 同款 canonical `SubId`/-1=none；`task_sub>=0` 时必须指向零参数 `Async` sub，绑定层派子任务，owner=新敌。**`drop_table` 在这里就展开成敌身上的 `drop_count[..]` 五槽**（敌死效果刀起表号退化成生成参数，此后无人读表号，见 520-522 号）；**P4-b**：表号越界（含负数——`as u16` 回绕后仍越界）→ **视同空表** + `contract_viol` +1 + `last_status=BAD_ARGS`，敌照建、**不 Fault**（原检查住 `settle::damage_enemy`，随掉落状态前移至此）） |
| 220 | `drop_item` | x,y,item_type | 道具句柄或 -1 |

`create_bullet` 走**丙方案**：`(xform_off, xform_cnt)` 指向本任务 locals 内打包槽
（每槽 3 字：`word0=(wait<<16)|(op<<8)`、`word1/2=args`，≤16 槽）；`xform_cnt=0` 哑弹；
`task_sub >= 0` 时其值是 canonical `SubId`，且必须指向零参数 `Async` sub；绑定层再派子任务
（owner=新弹）；appearance 查 `WorldTables.appearances`
定默认 radius/sprite。

### 3xx —— 弹操作（9）

**self owner 必须是 BULLET**，否则 Fault(0)（误用策略：响亮报错，非静默 no-op）。族内顺序照
`world/motion.rs` 的九连；九条都不押返回值。

```
300 set_bullet_speed  301 set_bullet_angle   302 turn_bullet
310 set_bullet_vel    311 set_bullet_ang_vel 312 set_bullet_accel  313 set_bullet_gravity
320 stop_bullet_fx
330 aim_bullet_at_player
```

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 300-330 | 弹 setter 族 | 按 motion.rs 九连 | —（owner 须为弹，否则 Fault） |

### 4xx —— 敌运动·表现状态（6）

对齐 ZUN ECL 的 `4xx`（`move` 族）。**self owner 必须是 ENEMY**，否则 Fault(0)；**敌句柄取自
owner、不占栈位**。五条都是 `world/motion.rs` 写 API 的薄封装，P4-b 校验（悬垂句柄 →
`STALE_HANDLE`；`easing >= 8` → `BAD_ARGS`）在世界层做过，绑定层不重复计数。

⚠️ **`dur`/`easing` 的收窄在派发臂、先于世界层**（D19，`ENGINE_VER` 12→13，2026-09-03）：
栈上是 `i32`，世界层收的是 `u16`/`u8`，这一步走 `u16::try_from` / `u8::try_from`，任一失败即
**整条 no-op + `contract_viol` +1 + `BAD_ARGS`**（与世界层那条 `easing >= 8` 是同一条腿，
判据前移）。**此前是裸 `as`**，于是 `easing = 256` 截断成 `0` 静默变线性、`easing = 264` 落 8
被正确拒掉——能不能拒取决于越界值模 256 落在哪里；`dur = -1` 变成"缓动 65535 帧"。
五条**共用同一个 helper**（`narrow_dur_easing`），判别式测试
`all_five_move_verbs_share_the_same_narrowing_leg` 逐条押着，漏掉任一入口即红。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 400 | `move_enemy_to` | **dur,x,y,easing** | —（owner 须为敌，否则 Fault(0)；参数序以 syscall.rs 为准，勿凭直觉写 x,y 在前。**P4-b 两码分开**：悬垂 owner 句柄 → no-op + `contract_viol` +1 + **`STALE_HANDLE`**；`easing >= 8` → no-op + `contract_viol` +1 + **`BAD_ARGS`**（不钳位）。`dur == 0` = 瞬移 + 硬停（写位置、清 `vx/vy` 并**回填** `speed`/`angle`、清在飞的位置插值），合法退化不计违约。**到点清速条件化**（敌人运动动词族刀 2026-07-31）：位置插值到点仅在黏滞位 `vel_touched == 0` 时清 `vx/vy`（清完同样回填作者视图）——判据是"脚本这一轮碰没碰过速度动词"，**不是**"速度插值还在不在跑"。⚠️ **本条是一次完整的运动接管**（终审裁定 2026-07-31）：武装时（`dur == 0` 与 `dur > 0` **两条路径**）既清黏滞位 `vel_touched`、**也清在飞的速度插值** `vel_active`——"表达过速度意图"这个事实都作废了，"意图正在执行中"更该作废；不清的话敌到点只停一帧就被旧 `from/to` 写回速度继续飘。要落地后继续飘，把 410-421 写在本条**之后**） |
| 410 | `move_vel`（敌人运动动词族刀 2026-07-31；ZUN `400 move` 族） | dur,angle,speed,easing | —（**self-only**，同 400 号 `move_to`：敌句柄取自 owner、**不占栈位**，owner 非 ENEMY → **Fault(0)**。四位全部真实压栈，派发臂逆序弹出 `easing,speed,angle,dur`。薄封装 `world::set_enemy_vel_polar`。**`dur == 0` = 立即设**（直接写 `speed`/`angle` 并刷 `vx/vy`，不置插值态，合法退化**不计违约**）；`dur > 0` 武装**极坐标空间**插值——逐帧插 `speed` 与 `angle` 再刷 `vx/vy`，故**匀速扫弧**、角度走**最短弧**（同 `transform.rs` 的 `STEP_ANGLE`）。**P4-b 两码分开**：悬垂 owner 句柄 → **整条 no-op** + `contract_viol` +1 + **`STALE_HANDLE`**；`easing >= 8` → **整条 no-op** + `contract_viol` +1 + **`BAD_ARGS`**。两条都不 Fault、**不钳位**（`enemy_vel_precheck` 与 400 号共用同一份判据，两码也逐条同 400 号）。四条动词（410-421）任一成功执行都置敌身上的黏滞位 `vel_touched=1`（**`dur == 0` 的瞬时腿同样置位**），400 号 `move_to` 武装时把它归零——见 400 号的"到点清速条件化"。**`dur == 0` 还会解除在飞的速度插值**（`vel_active = 0`）：不解除的话瞬时值当帧生效、次帧就被仍在飞的插值器按旧 `from/to` 覆盖回去，脚本的"急停"静默失效 |
| 411 | `move_vel_xy`（同刀） | dur,vx,vy,easing | —（同 410 号的 owner 门禁 / P4 / `dur==0` 口径，只有插值空间不同：`dur > 0` 时在**笛卡尔空间**插值——`vx`/`vy` 各自线性插、再回填 `speed`/`angle`。**这不是 410 号的语法糖**：同一对端点两条走的轨迹不同（笛卡尔是速度矢量直线穿过、中途速率掉，线性缓动即恒定加速度；极坐标是匀速扫弧）。把这条实现成"转极坐标再插"就退化成 410 号了，`integrate.rs` 的招牌判别式 `polar_and_cartesian_velocity_interpolation_take_different_paths` 就是钉这个的。薄封装 `world::set_enemy_vel_cart`） |
| 420 | `move_angle`（同刀） | dur,angle,easing | —（**只转向、速率一字不动**：终点 = `(当前 speed, 目标 angle)`，走极坐标空间。3 位压栈。其余口径同 410 号） |
| 421 | `move_speed`（同刀） | dur,speed,easing | —（**只调速、方向一字不动**：终点 = `(目标 speed, 当前 angle)`，走极坐标空间。3 位压栈。其余口径同 410 号） |
| 430 | `set_anm_state`（表现契约 v2，2026-09-07） | state | —（**self-only**，owner 非敌 → Fault(0)；悬垂 owner 句柄 → no-op + `contract_viol` + `STALE_HANDLE`。写 `anm_state = state as u16` 并**无条件**盖 `anm_state_frame = 当前帧`——同状态重设 = 重播（ZUN `anmInterrupt` 重触发语义的电平版）。世界不解释状态号；表现层按 `(sprite, anm_state, frame − anm_state_frame)` 选帧，见 `render-contract.md` §7。**不是** ZUN 的 `anmSetSprite`：运行期换贴图不进核，换形态用状态号映射） |
| 440 | `set_invuln`（boss 换段刀 2026-09-14，敌判定族） | frames | —（**self-only**，owner 非敌 → Fault(0)；悬垂 owner 句柄 → no-op + `contract_viol` + `STALE_HANDLE`。覆写 `enemies.invuln`，0 = 取消；期间伤害结算跳过、不发 `EVT_SHOT_HIT_ENEMY`，相位 5 每帧递减。`frames ∉ [0,65535]` → 整条 no-op + `contract_viol` + `BAD_ARGS`） |
| 441 | `set_hitbox`（同刀） | r | —（self-only 同 440。写**体碰半径** `radius`（碰撞行 3）；`Fx` raw，钳 `[0, MAX_ENTITY_RADIUS]`，钳了计 `contract_viol`，同 `create_enemy` 口径） |
| 442 | `set_hurtbox`（同刀） | r | —（self-only 同 440。写**受击半径** `hurtbox`（碰撞行 4/7）；钳制同 441） |
| 443 | `set_enemy_flag`（同刀） | flag,on | —（self-only 同 440。`on != 0` 置位否则清位；`flag` 须为 `ENEMY_NO_BODY(2) \| ENEMY_KILLALL_EXEMPT(4)` 的**非空子集**，含 `ENEMY_DYING` 或未知位或 0 → 整条 no-op + `contract_viol` + `BAD_ARGS`。`ENEMY_NO_BODY` 让碰撞行 3 跳过、仍吃弹） |

### 5xx —— 局面·记账·道具（14）

对齐 ZUN ECL 的 `5xx`（drops）。owner 类别**默认无限制**（STAGE 任务常发）——**例外是
`drop_clear`/`drop_add`/`drop_items`/`die` 四条，self owner 须为敌，否则 Fault(0)**，逐条见各行。
钳位（分数/残机/bomb/火力）是**正常语义**，不计 `contract_viol`、不 Fault。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 500 | `add_score`（整局流程刀，见 consts.rs 5xx 族） | delta | —（自机 0 记分；`delta` 允许负值扣分，结果**饱和钳** `[0, u64::MAX]`——扣穿停在 0、加满停在上限，不回绕；不做参数收窄，不 Fault，owner 类别无限制） |
| 510 | `add_lives`（B20） | delta | —（自机 0；`delta` 允许负，`saturating_add` 后**双边钳** `[0, u8::MAX]`——扣穿停 0、加满停 255，不回绕不 panic；**钳位是正常语义**，不计 `contract_viol`、不 Fault（同 `add_score` 口径）；不做参数收窄，owner 类别无限制） |
| 511 | `add_bombs`（B20） | delta | —（同 510，写 `bombs`（X 键库存：Chronos 停止 / Classic bomb），钳 `[0, STOP_STOCK_MAX=5]`（玩法刀）。513 号原 add_time_stops 已随玩法刀 2026-09-14 退役，号不复用） |
| 512 | `add_power`（B20） | delta | —（同 510，写 `power`，但上钳是 `items::POWER_MAX`=**400**（显示 4.00）**而非 `u16::MAX`**——越过它 `power_tier` 档位索引 OOB） |
| 520 | `drop_clear`（敌死效果刀） | — | —（**0 参**；把 self 敌的 `drop_count[..]` 五槽清零。**owner 须为敌**，否则 Fault(0)（同 `move_enemy_to`）；悬垂 owner 句柄 → no-op + `contract_viol` + `STALE_HANDLE`，不 Fault。参照 ZUN `dropClear`(506)） |
| 521 | `drop_add`（敌死效果刀） | type,n | —（逆序弹栈 `n, type`；给 self 敌的待掉落计数**增量**加 `n` 颗 `type`，**只增不减**（人类裁定，清空用 520）。`type` 收窄 `[0, items::ITEM_TYPE_COUNT)`，越界 → no-op + `contract_viol` + `BAD_ARGS`，**不 Fault**（P4-b）；`n` **先钳** `[0, u8::MAX]`（负 n 视同 0）**再 `saturating_add`** 到计数上——两步都要，只钳不饱和会在近 255 时 debug panic，只饱和不钳会让负 n `as u8` 回绕。owner 须为敌，否则 Fault(0)。参照 ZUN `dropExtra`(507)） |
| 522 | `drop_items`（敌死效果刀） | — | —（**0 参**；立刻把 self 敌的待掉落计数撒出去（按类型编号升序逐颗 `spawn_drop`，**消耗世界 RNG**）。**吐完不清空计数**（人类裁定，照 ZUN 字面）——故 `drop_items(); die();` 掉**双份**，作者自负；**不加分、不发 `EVT_ENEMY_DIED`、不发 `REQ_ENEMY_DEATH`、不标 `ENEMY_DYING`**；对已 dying 的敌照撒不误（无幂等门禁，与 530 不同）。池满走 `spawn_drop` 自身的 P4-a 逐颗降级。owner 须为敌，否则 Fault(0)。参照 ZUN `dropItems`(509)） |
| 530 | `die`（敌死效果刀） | — | —（**0 参**；对 self 敌跑**完整死亡效果**（`world::settle::kill_enemy`）：`hp = hp.min(0)` → 标 `ENEMY_DYING` → 撒掉落 → `enemies.score` 记进自机 0 → `EVT_ENEMY_DIED` → `REQ_ENEMY_DEATH`。**幂等**：已 dying → 直接返回。**只标记不回收**，相位 9 cleanup 才收尸——当帧体碰仍成立。表层 `die()` 由 codegen 降低成 **`SYS 530` + `OP_KILL_SELF` 两条指令**，故调用它的任务立即终止（人类裁定），后续语句不执行；**手写字节码只发 `SYS 530` 不会终止任务**。owner 须为敌，否则 Fault(0)。参照 ZUN `die`(561)——ZUN 那条还经 `setDeath`(556) 间接一层，留给 `death_script` 通电那一刀） |
| 531 | `kill_all_enemies`（boss 换段刀 2026-09-14） | mode | —（owner 无限制。按池索引升序遍历存活敌，跳过：调用任务的 owner 敌、已 `ENEMY_DYING`、带 `ENEMY_KILLALL_EXEMPT`。`mode = KILL_SILENT(0)` 只置 dying（同 D9 退场：不掉落不加分不发事件）；`KILL_DIE(1)` 逐只 `kill_enemy`（同 530 `die`）；其它 → 整条 no-op + `contract_viol` + `BAD_ARGS`。杀到符卡槽绑定 boss → 当帧相位 7 按血线路径结算） |
| 540 | `clear_bullets`（B19） | — | —（**0 参**；调 `create_field(field::fullscreen_clear_field())` 铺一个覆盖全场、`life=1`、`FIELD_CLEAR_BULLETS` 的作用区，当帧相位 6 生效——消弹转星星与 `EVT_FIELD_CLEARED` 都是消弹区机制白送的，syscall 层零新逻辑；不做参数收窄；**P4-a 两处**：(a) field 池（cap 16）满 → 走 `create_field` 自身降级（NULL + `diag.pool_full[POOL_FIELD]` +1），**不 Fault**；(b) **消弹转星星是 1:1，而道具池 cap 1024 < 弹池 cap 8192** ⇒ 一次消掉的弹多于道具池余量时，多出的星星**生不出来**：逐颗计 `diag.pool_full[POOL_ITEM]`、**循环有界不短路**（判别腿 `star_pool_full_counts_every_missing_star`），弹照消不误。**这是已知设计边界不是债**——F12 实测 demo 收卡一帧 626 颗弹，池还是 512 时四个难度档全溢出，抬到 1024 才盖住（`ENGINE_VER` 13→14）；owner 类别无限制） |
| 541 | `clear_bullets_at`（boss 换段刀 2026-09-14） | x,y,r,stars | —（owner 无限制。铺一个中心 `(x,y)`、半径 `r`（`Fx` raw，钳制走 `create_field`）、`life=1`、无伤害的清弹区；`stars == 0` 带 `FIELD_NO_STAR`：弹照标 `BULLET_CLEARED`、照计 `EVT_FIELD_CLEARED`，但不转星星。池满走 P4-a。扩张消弹波 = 脚本每帧调一次加大 `r`） |
| 550 | `bgm` | id | —（写表现锚点 `bgm_id` + 发 `REQ_BGM`；`id` 收窄 `0..=65535`，越界 → no-op + `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不 Fault**（P4-b），owner 类别无限制） |
| 551 | `bg` | id | —（同上，写 `bg_id` + 发 `REQ_BG`；同一收窄/no-op 口径） |
| 552 | `bg_phase` | n | —（写 `bg_phase`，同时把 `bg_phase_frame` 盖为当前帧，再发 `REQ_BG_PHASE`；`n` 同上收窄/no-op 口径） |
| 560 | `time_stop_player`（自机能力刀，ECL 演出方向） | frames | —（写 `freeze_left[1]` ⇒ 冻 A+B——自机不能移动/发新弹，已在场上的自机弹也冻住，敌方照常行动；碰撞判定不冻，弹幕仍会打中自机；`frames=0` 即**立即解除**，天然的取消 API；重入取**覆盖**（后写为准），不取最大不叠加；`frames` 收窄 `u16::try_from`，越界（负值或 >65535）→ **整条 no-op** + `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不钳位、不 Fault**（D19 判例）） |

### 6xx —— shooter（预存发射参数集）（15）

对齐 ZUN ECL 的 `6xx`（`et*` 弹管理器）。每任务 **4** 个编号槽
（`ecl::shooter::SHOOTERS_PER_TASK`）；**`600-660` 共用一份 `id` 判据**（`shooter_mut`）：
`id < 0 || id >= 4` → no-op + `contract_viol` +1 + `BAD_ARGS`，**不 Fault**（槽号笔误是常见错，
降级比杀任务有用）。owner 类别无限制。**前 14 条只写字段、不校验**——appearance 在册与否、
xform 区间、sub 号在册统统留到 `sh_fire`(660) 那一刻查（同 `fire` 的先验后建）。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 600 | `sh_reset`（shooter 刀） | id | —（把本任务第 `id` 号发射器槽整个抹回 `ShooterSlot::default()`：**1 角×1 层单发**、其余字段全零、`task_script=SH_NO_TASK`、`on_fire_req=0`。**600-660 共用的 `id` 判据**（`shooter_mut`）：`id < 0 \|\| id >= SHOOTERS_PER_TASK`(=4) → no-op + `contract_viol` +1 + `BAD_ARGS`，**不 Fault**（P4-b：槽号笔误是常见错，降级比杀任务有用）；判据在**参数全部弹完之后**才施加，故越界腿与成功腿栈效应一致。owner 类别无限制。参照 ZUN `600 etNew`） |
| 610 | `sh_sprite`（shooter 刀） | id,**shape,color** | —（表层三参，`shape`/`color` 在**编译期**折叠成单个 appearance（`builtins::fold_start("sh_sprite") == 1`——起点是 **1** 不是 0，头一位是槽号），故字节码层只压 **2** 个值。收窄用 `u16::try_from(..).unwrap_or(u16::MAX)`——**保号越界性**：负值/超 u16 一律落 `u16::MAX`（远超表长），660 号开火时照样查不到而 Fault；**不得**改成 `clamp(0,u16::MAX)`，那会把负值洗成 **0**（`appearances[0]` 在册且 valid）、越界值再也拒不掉。**在册与否留到开火时查**（同 `fire` 的先验后建）。参照 ZUN `602 etSprite`） |
| 620 | `sh_offset`（shooter 刀） | id,x,y | —（出弹点**相对 owner** 的直角偏移（Fx raw）；写 `off_x/off_y` 并**清** `SH_ABS_OFFSET` 位。与 621 写同一对字段（后写的赢），与 622 的极坐标偏移**永远叠加**。无收窄（Fx raw 全域合法）。参照 ZUN `603 etOffset`） |
| 621 | `sh_offset_abs`（shooter 刀） | id,x,y | —（同 620 但**置** `SH_ABS_OFFSET` 位 ⇒ 开火时基点取世界原点、不跟随 owner。两条互为反向，这是有意设计。参照 ZUN `628 etOffsetAbs`） |
| 622 | `sh_offset_rad`（shooter 刀） | id,angle,r | —（极坐标偏移：`polar_ang`（低 16 位取 BAM，同 `OP_SINB` 惯例）+ `polar_r`（Fx raw）。**不碰** `SH_ABS_OFFSET` 位；开火时 `origin = 基点 + (off_x,off_y) + polar_to_vec(polar_r, polar_ang)`——**叠加不覆盖**（ZUN 626 明写 stacks）。参照 ZUN `626 etOffsetRad`） |
| 623 | `sh_dist`（shooter 刀） | id,d | —（出生后沿**各自角度**推出去的距离（Fx raw）——逐颗方向不同，**不是整环平移**；`d=0` 时 `polar_to_vec` 恒返 (0,0)，开火路径不分支。判别腿 `dist_pushes_each_bullet_along_its_own_angle`。参照 ZUN `627 etDist`） |
| 630 | `sh_angle`（shooter 刀） | id,angle0,step | —（两值都取低 16 位存成 `Angle`。`angle0` = 基准角；开了 `SH_AIMED` 时它是**相对自机方向的偏移**。`angle_step` 的含义**随 `SH_RING` 转义**：fan 下是逐弹增量、ring 下是**逐层**偏移。⚠️ 存的是 `Angle`（底层 u16），开火侧读回时**必须 `as i16 as i32` 符号扩展**——`((n−1)·step)/2` 的除 2 不与 mod 65536 交换，零扩展会让**负 step × 偶数路**整把扇形偏 180°（复审 Critical，钉在 `fan_centering_handles_even_ways_with_negative_step`）。本仓家规同侧：`create_bullets_batch` 形参就是 `angle_step: i16`、`Angle::add_delta` 也收 i16。参照 ZUN `604 etAngle`） |
| 631 | `sh_speed`（shooter 刀） | id,speed0,step | —（`speed0`/`speed_step`（Fx raw）；第 j 层速度 = `speed0 + j×speed_step`，开火时用**累加器**推进（与 `batch` 的 `cur_speed + speed_step` 同款，保证等价测试逐位相同）。无收窄。参照 ZUN `605 etSpeed`） |
| 632 | `sh_count`（shooter 刀） | id,n_angle,n_speed | —（网格规模。两值各自 `clamp(0, u8::MAX)` 后存 u8——**先钳再 `as`**（裸 `as u8` 会让负数回绕成大正数）；钳位是正常语义，不计 `contract_viol`。零与超容的处置在 660 号。参照 ZUN `606 etCount`） |
| 640 | `sh_aim`（shooter 刀） | id,on | —（`on != 0` 置 `SH_AIMED`、否则清。开火时才解析自机方向（不是设的时候算死，判别腿 `aim_resolves_at_fire_time_not_at_set_time`），基点是**出弹点**而非 owner 位置（判别腿 `aim_is_measured_from_the_fire_origin_not_from_the_owner`）。"瞄谁"与 120 号 `aim_player` 同一口径（`WorldBody::aim_target`——从**出弹口**看过去最近的可瞄自机，一个可瞄的都没有则回退 `players[0]` 的最后坐标：发射这条路必须产出一个角度。判别腿 `shooter_aim_skips_unaimable_player` / `shooter_aim_falls_back_to_p0_when_none_aimable`）。参照 ZUN `607 etAim` 九值 aimmode 枚举的 aimed 半——D-6 把那个枚举塌成 `aimed`/`ring` 两个正交布尔） |
| 641 | `sh_ring`（shooter 刀） | id,on | —（`on != 0` 置 `SH_RING`、否则清。开则 `n_angle` 颗**自动均分整周**、`angle_step` 转义成逐层偏移；关则 fan（逐弹增量 + **以基准方向为中心**对称展开）。参照 ZUN `607 etAim` 的 ring 半（同上 D-6）；ZUN 的 mode 4/5「offset ring」在此冗余——ring 下 `angle_step` 本就是逐层偏移，「错开半步」写 `sh_angle(id, base, (32768/n) as angle)` 即可，不必单开模式） |
| 650 | `sh_xform`（shooter 刀） | id,xform_off,xform_cnt | —（表层 `XformRef` 由 codegen 降低成 `(off, cnt)` **两个**栈值，`cnt == 0` = 无 xform。此处只做**存储收窄**的饱和钳（`off` → u16、`cnt` → u8，防负值/超宽裸 `as` 回绕成看似合法的小区间），**不校验区间**——`cnt ≤ 16` 与 `off + cnt*3 ≤ LOCALS` 在 660 号开火时查（同 `create_bullet` 的丙方案边界口径）。数据住**本任务 locals**，由 codegen 在 sub 入口一次性 staging、`slots` 的调用图着色保证区间不被复用 ⇒ **跨帧延迟读是安全的**（钉在 `xform_survives_a_frame_boundary_between_set_and_fire`）。⚠️ 开火时是**每颗弹**自有一个段拷贝，见 660 号的段池压力（同 651 号之于任务槽）。参照 ZUN `609-612 etEx*`） |
| 651 | `sh_task`（shooter 刀） | id,task_sub | —（表层 `SubRef` 降低成一个栈值（canonical `SubId` 或 -1=none）。负值 → `SH_NO_TASK`(0xFFFF)；超 u16 的号同样降级成"不挂"（P4-b）。**此处不校验号在册**——设的时候还没建弹，校验在 660 号开火时做。⚠️ 开火时是**每颗弹**派一个任务，见 660 号的任务槽压力。**ZUN 的 `et*` 族无此参**——这是本仓扩展，与 `fire`/`spawn_enemy` 的 `task` 参同构） |
| 652 | `sh_req`（shooter 刀） | id,req_id | —（开火时顺带发的通道 B 请求 id；`clamp(0, u16::MAX)` 收窄，**`0` = 不发**（ZUN 608 的 sound1 归并进通道 B）。载荷布局见 660 号。<br>⚠️ **本族唯一不"保号越界性"的 setter，与 `emit_req`(720)/`bgm`(550) 等同 id 空间的兄弟口径不同，是有意的**：那些兄弟对越界 id 是 **no-op + `contract_viol` + `BAD_ARGS`**，而本条**钳**——`sh_req(0, 100000)` 存下 65535，于是 660 号会发出一个脚本从没要求过的 id。两条理由：① 危害有界——`reqs` 是 `#[checksum(skip)]` 的**纯输出缓冲**，请求分发器对未知 id 是 warn-and-ignore，坏 id 顶多是"少放一个音效"，不像 610 号 `sh_sprite` 那样一旦洗掉越界性就再也拒不掉隐形弹；② "修好"它要动 `contract_viol`，而**那个是进校验和的**——改一个纯表现通道的参数校验去动确定性状态，代价方向反了。要在设的那一刻就拒坏 id，用 `emit_req` 自己发。参照 ZUN `608 etSound` 的 sound1；sound2 与 ZUN 的音效通道概念一并归并进通道 B，不单列） |
| 660 | `sh_fire`（shooter 刀） | id | —（**无返回值**，人类裁定 D-8：本语言要求值必须消费，有返回就得写 `_ = sh_fire(0);` 而开火是循环里最高频的语句——**别"补全"成返回实发数**。用槽 `id` 的参数造弹，七步：① 退化网格 → ② appearance → ③ xform 区间 → ④ 挂弹任务号 → ⑤ 原点 → ⑥ 基准角 → ⑦ 网格循环，**一切拒绝都发生在任何世界写之前**（同 `create_bullet` 的先验后建）。<br>**P4 处置逐条**：`id` 越界 → no-op + `contract_viol`（同 600-652）；`n_angle==0 \|\| n_speed==0 \|\| n_angle*n_speed > BulletPool::CAP` → **不发 + `contract_viol` + `BAD_ARGS`，不 Fault**（对齐 `create_bullets_batch` 的退化网格口径）；appearance 越界或 `!valid` → **Fault(0)**；`xform_cnt > 16` 或 `off + cnt*3 > LOCALS` → **Fault(0)**；`task_script` 不在册 / 非零参 `Async` sub → **Fault(0)**；弹池满 → **短路本次开火的剩余部分**（同 `batch` 的 `'grid`，同相位无回收 ⇒ 后续必然同败），已发的留着；**xform 段池满**（只在 `sh_xform` 非空时可达）→ **同样短路剩余部分** + `pool_full[POOL_XFORM]` +1 + `STATUS_POOL_FULL`，**不 Fault**（P4-a）——`create_bullet_with_xform` 是"先段后弹"，段分配不到就直接返回 NULL，`'grid` 短路不区分是弹池还是段池，但**计数器是两个、失败模式是两条**；**段消耗账**：`sh_xform` 非空时每颗弹自有段拷贝 ⇒ 一次 `sh_fire` 吃 `n_angle × n_speed` 个段（段池共 **2048**），`sh_xform` + `sh_count(0, 28, 5)` = **140 段**，与 `batch` 的段消耗账逐字同一件事（见 [`xform-ops.md`](xform-ops.md)"消费入口"）；挂弹任务池满 → **弹保留、任务丢** + `pool_full[POOL_TASK]` +1，**不 Fault**（P4-a，同 `fire`）。<br>网格序 = **角度外层、速度内层**（= 池槽分配序，I4）。fan：`base + i·step − ((n−1)·step)/2`（**居中**，D-7）；ring：`base + (i×65536)/n + j·step`（逐颗算、余数均摊 ⇒ 精确闭合；`i ≤ 254` 故 i32 不溢出）。`on_fire_req != 0` 时发 `emit_req(on_fire_req, [origin_x, origin_y, appearance, **实际创建数**, 0, 0])`——`args[3]` 是实发数**不是**请求数（池满时要能区分）。owner 类别无限制。参照 ZUN `601 etOn`） |

### 7xx —— 控制·事件·符卡·globals（10）

剩下的控制面。`get_var`/`set_var` 的段纪律见下方"globals 段纪律"；`spell_begin`/`spell_end`
的完整口径见下方"符卡计器"。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 700/701 | `get_var` / `set_var` | slot / slot,val | 值 / —（`get_var` 两段皆无限制；`set_var` 写**系统段**
  `slot<16` → no-op + `diag.contract_viol` 计数，**不 Fault**——见下方"globals 段纪律"及
  `world::GLOBALS_SYS_SEGMENT` 文档） |
| 710 | `pulse_signal` | ch | — |
| 720 | `emit_req` | id a0 a1 a2 a3 a4 a5 | — |
| 721 | `fx_at`（表现契约 v2） | x,y,kind,param | —（owner 无限制。= `emit_req(REQ_FX_AT, [x raw, y raw, kind, param, 0, 0])` 的钉死布局版，对应 ZUN `anmPlayPos`；`kind`/`param` 引擎不解释，归内容包与壳侧约定。即发即忘类；缓冲满走 D12） |
| 722 | `fx_on`（表现契约 v2） | kind,param | —（**self-only**，owner 非敌 → Fault(0)。发 `REQ_FX_ATTACHED, [index, gen, kind, param, 0, 0]`，句柄取自 owner 敌、**裸 index/gen 两位**不打包；壳侧按 `(index, gen)` 每帧跟随（桥面 `entity_pos`）、句柄失效即自毁。对应 ZUN `anmPlay`。即发即忘类） |
| 723 | `stage_clear`（壳子刀 2026-09-07） | stage | —（owner 无限制。发**事件** `EVT_STAGE_CLEARED{data0 = stage}`——流程信号走通道 A 事实流，不发通道 B 请求；表层 codegen 在它后面追发 `PUSHI 1; WAIT` 让出一帧，故 `stage_clear(n);` 之后的语句在宿主放行后的**第一帧**才执行。手写字节码只发 `SYS 723` 不让出帧。取代 `emit_req(REQ_STAGE_CLEAR, …)` 挂牌协议——那条不让出帧、下一关开头会在挂牌同一帧跑掉） |
| 730 | `boss_set` | slot,hp_ratio,spell_id,timer,phase_left,active | —（enemy 字段写 NULL，见 boss_ui 契约） |
| 740 | `spell_begin`（符卡机构，见下方"符卡计器"） | slot,spell_id,pattern_sub,time_limit,bonus0,flags,hp_threshold | —（owner 须为敌，否则 Fault） |
| 741 | `spell_end`（符卡机构，见下方"符卡计器"） | — | —（owner 绑定槽走 HP 路径结算；无绑定槽 → no-op，重复调用安全） |

- **`emit_req`（720）**：通道 B 渲染请求（`docs/ecl-lang.md`"渲染请求"节）。id 收窄 P4-b：
  栈值超出 `0..=65535` → no-op + `contract_viol` + `BAD_ARGS`，不 Fault；缓冲满走 D12
  （丢弃 + `TRUNCATED` + `diag.reqs_dropped`）。无 owner 类别限制（STAGE 任务可发）。

### shooter 族（600-660）对 ZUN `et*` 的**有意缺口**

号表是白名单，"ZUN 有而这里没有"永远是有意的，不是漏排。写在这里免得将来某次
"补全性"清扫把它们静默加回来（spec `2026-07-31-ecl-shooter-design.md` §3 的十条裁定）：

- **`614 etCopy`（拷贝一个发射器槽到另一个）——不做**（裁定 D-9）。两条理由：ZUN 自己的
  文档就自陈这条 **"partially broken"**；且 `SHOOTERS_PER_TASK` 只有 **4**，真要复制一个
  槽，手写几行 setter 就完事，不值得为它多一个进契约、要冻结、要测的号。**想加回来先
  推翻 D-9**。
- **`617-625` 难度分档族——不在本刀**（D-10），归「难度分档」独立一刀，不是否决。
- **`608 etSound` 的 sound2**——与 ZUN 的音效通道概念一并归并进通道 B（见 652 号），不单列。
- **`607 etAim` 的九值 aimmode 枚举**——D-6 塌成 `sh_aim`/`sh_ring` 两个正交布尔（640/641 号）：
  mode 4/5 在两布尔下冗余，**mode 6/7/8 的随机模式不做**（另见 `follow-ups.md` D13）。

### 8xx —— 激光（10）

激光是一等实体（射线原点 + 方向 + 射线上 `[start, end]` 一段，spec
`2026-09-25-laser-pool-design`）。`laser()` 建一条并返回**打包激光句柄**（编码同敌号：
`((generation & 0x7FFF) << 16) | index`，`-1` 为唯一无效哨兵；槽复用可辨）；其余 `lz_*` 都拿
这个句柄操作，参数**逆序弹出**。句柄失效（已回收 / 代际不符）→ 写 API 一律 no-op +
`contract_viol` +1 + `STALE_HANDLE`，`lz_alive` 只读不计数（P4-b）。owner 类别无限制。

| 号 | 名 | 参数（压栈序） | 返回 |
|---|---|---|---|
| 800 | `laser`（激光池刀 2026-09-25） | color,x,y,angle,len,width,warn,active,fade | 打包句柄或 -1（`color ∉ 0..=15` → **Fault(0)**，先验后建、不建半成品，同 `fire` 坏外观；`warn/active/fade` 出 `[0,65535]` → 钳位 + `contract_viol` +1；形态一 `start=0`、`end=start_len=len`、`speed=omega=0`、`flags=0`，其余派生字段交给世界侧；池满走 P4-a：押 -1 + `pool_full[POOL_LASER]` +1，**不 Fault**；`sprite` 存 `color`） |
| 801 | `lz_speed` | lz,speed,start_len | —（形态二：设速率与近端长度，并把 `end` 重置为 `start` 从近端长出去；两字段世界层双边钳 `[0,LASER_LEN_MAX]`） |
| 802 | `lz_start` | lz,s | —（近端留空，原作第 4 关 `start=64`；世界层钳 `[0,LASER_LEN_MAX]`，`start > end` 时把 `end` 抬到 `start`） |
| 803 | `lz_omega` | lz,a | —（持续转动速率；池字段是 `i16` BAM/帧，栈值出 i16 → 钳到 i16 界 + `contract_viol` +1） |
| 804 | `lz_rotate` | lz,a | —（一次性转 `a`，回绕加） |
| 805 | `lz_aim` | lz,off | —（角度 = 指向自机 0 的 `atan2` + `off`） |
| 806 | `lz_anchor` | lz,enemy,ox,oy | —（挂到敌号上；`enemy = -1` → `EnemyHandle::NULL` 解除挂靠。其它敌号一律经 `resolve_enemy_handle` 解析：命中则用池里的**完整 u16 代际**重建句柄（I-3，低 15 位复刻会让高代际静默脱钩）；解析失败 → 不挂靠 + `contract_viol` +1 + `STALE_HANDLE`。偏移世界层双边钳 `±LASER_COORD_MAX`，存活敌立即吸附到 `敌位置 + 偏移`） |
| 807 | `lz_origin` | lz,x,y | —（直接设原点并**解除挂靠**；坐标双边钳 `±LASER_COORD_MAX`） |
| 808 | `lz_cancel` | lz | —（`state < 2 → 2`、`timer = 0`；已收缩则 no-op 但仍成功） |
| 809 | `lz_alive` | lz | 1/0（只读，不计数） |

- 时序口径（全计划统一）：相位 5 里先按 `timer >= 时长` 判切换（切换时 `timer = 0`），再
  `timer += 1`——`warn == 0` 出生帧即生效，预警恰好 `warn` 帧不判定、生效恰好 `active` 帧判定。
- 激光只在相位 5 回收（fade 结束、`start >= 640`、`active` 结束且 `fade == 0`）；相位 7 的
  取消只切状态，`fade == 0` 的激光在下一帧相位 5 回收。带 `FIELD_CLEAR_BULLETS` 的 field
  碰到激光线段也取消（碰撞行 10）。

## 符卡计器（syscall 130/131/740/741 + `wait_spell` / `phase_begin` 糖；spec 2026-07-24，boss 换段刀 2026-09-14 扩）

记账（计时/衰减/超时/破卡/`boss_ui` 喂送）全归引擎 `SpellState` 机构（settle 相位符卡趟），
三条 syscall 是脚本唯一的操作面；表层参考见 [`ecl-lang.md`](ecl-lang.md)"符卡"节。

- **`spell_begin`（740）**：参数**逆序弹栈**（`threshold, flags, bonus0, time_limit,
  pattern_ref, spell_id, slot`）；owner 须为 `ENEMY`，否则 Fault(0)（同 `move_enemy_to`
  误用策略）。`pattern_ref`（`SubRef`，负值=none）**先查后建**：越界/非零参 `Async` 号 →
  Fault(0)（零副作用，同 `create_bullet`/`SPAWN` 坏号口径）；查通过后才调用世界层
  `spell_begin_internal`，其 P4-b 拒收条件（no-op + `contract_viol` 计数，不 Fault）：
  `threshold<0`、`bonus0<0`、`time_limit<=0`、槽越界、槽已 `active`、owner 已绑定另一槽。
  成功：定格 bonus 衰减参数（地板/速率）+ 记 `hp_start` + `pattern_ref` 非 none 时照
  `create_bullet` 的 task-spawn 样板 spawn 模式任务（owner=本敌，`spell_bound=slot+1`
  绑定该卡槽，随卡生死见下）+ 发 `EVT_SPELL_DECLARED` 事件 + `REQ_SPELL_DECLARE` 请求。
- **`spell_end`（741）**：无参，owner 须为 `ENEMY`（Fault(0) 同上）。owner 有绑定槽 → 走
  HP 路径结算（资格在 → CAPTURED 付 `bonus_now`；资格失 → FAILED）；无绑定槽 → no-op
  （不计数，重复调用安全——脚本可以无条件调它当"确保收尾"）。
- **`spell_timer`（130）**：无参，读族。owner 非 `ENEMY` 或无绑定槽 → **押 -1**（同
  `self_hp`/`self_hp_max` 误用降级口径——不 Fault，方便脚本用 `>= 0` 判活）；有绑定槽 →
  该槽 `frames_left`。这个 `-1` 判据正是 `wait_spell()` 糖的展开条件。
- **`spell_result`（131，boss 换段刀）**：1 参 `slot`，读族，owner 无限制。押 `WorldBody.spell_last_result[slot]`：
  0 还没结束过 / 1 血线 / 2 超时 / 3 手动；槽越界押 0 + `contract_viol` + `BAD_ARGS`。只由结算写，`spell_begin` 不清。
- **非符段 `SPELL_NONSPELL`（flags bit2）**：`spell_begin` 不发 `EVT_SPELL_DECLARED`/`REQ_SPELL_DECLARE`，bonus 三字段写 0
  （传入值忽略、不计违约），`SPELL_SURVIVAL` 位忽略；结算不付分、不发 CAPTURED/FAILED/`REQ_SPELL_RESULT`，改发
  `EVT_PHASE_ENDED`（12，`data = [spell_id, cause]`）。表层糖 `phase_begin` 纯前端注入常量，降低到 740。
- **结算顺序**（`settle_one_spell(slot, cause)`，`cause` = 1 血线 / 2 超时 / 3 手动）：① 超时时绑定 boss 存活且非 dying →
  `hp = min(hp, hp_threshold)`；② 付分与事件；③ 除非 `SPELL_NO_CLEAR` 铺全屏清弹区，**超时路径带 `FIELD_NO_STAR`**（不转
  星星）；④ 写 `spell_last_result`；⑤ 槽与 `boss_ui` 清零。

**`wait_spell()` 语法糖**（纯编译器前端，零 VM/字节码改动）：`lang::parse` 直接把
`wait_spell();` 展开成等价的 `Stmt::While` 子树，等同于脚本作者手写
`while spell_timer() >= 0 { wait(1); }`——**没有专属 op、没有专属 syscall**，codegen 拿到
展开后的 AST 和手写的 while 一视同仁，产出逐字节相同的 `EclImage`（判别测试见
`stg-ecl-compiler::lang::codegen::tests`）。字节码层看到的永远只是普通的 `JZ`/`SYS 130`/
`WAIT` 组合，本表其余各条纪律（返回值必须消费、死循环需要 `WAIT` 等）照常适用。

## globals 段纪律（甲案，M1.5）

`globals`（`GLOBALS_CAP=1024` 槽）拆两段，边界 `world::GLOBALS_SYS_SEGMENT=16`：

- **系统段** `[0, 16)`：只准 **game 层经世界 API** `WorldBody::set_var` 写（场景搭建代码，如
  `stg-harness` 建场时 `world.body.set_var(GVAR_RANK, ..)`）；**脚本经 `SYS 701 set_var` 写这段
  一律 no-op**——真槽值不变 + `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不 Fault**
  （P4-b 脚本作者违约的确定性安全结果，调度层不杀任务，脚本继续往下执行）。已命名槽：
  `GVAR_RANK=0`（难度，见下方"作者须知"）——**取值域 `0..=4`**
  （`RANK_EASY`/`RANK_NORMAL`/`RANK_HARD`/`RANK_LUNATIC` + 预留的 `RANK_EXTRA`，五个具名
  常量注入脚本，见 `consts.rs`）。`World::new_game_at` 对越界 `rank` 返
  `TaskStartError::RankOutOfRange`（**拒绝而非钳位**——`rank` 是回放/握手身份
  `(seed, rank, start, loadout, image)` 的一员，钳过的值会让重放契约变得可疑），写槽发生在
  校验之后，故槽里的值恒在域内。
- **自由段** `[16, 1024)`：脚本读写皆无限制。
- `SYS 700 get_var`（脚本读）**两段皆不受限**——只有脚本**写**系统段被挡，读不受影响。

## Fault 码（确定性报错：杀任务 + `EVT_TASK_FAULT{a_index=任务号, data=[码, script]}` + `diag.task_faults`）

| 码 | 含义 |
|---|---|
| 0 | 未知 op / **坏操作数值**（locals 槽号越界、坏 syscall 号、坏 owner 类别等复用此码）。**未知 op、非法跳转目标、CALL/SPAWN 目标不对、局部下标越界、坏 syscall 号这几类，正经过 `try_from_parts` 的镜像在加载时就已被拒（`ImageBuildError::BadCode`，见上方"加载时代码校验"）——运行时还会撞见码 0 的，只剩 syscall 内部依赖运行时值的动态检查（如坏 owner 类别）** |
| 1 | pc/跳转目标/操作数字越界。**跳转目标越界这条同样已提前到加载时拒绝**；运行时仍会撞见的是取指越界与操作数越界——存档能把 task pc 带到任意位置，镜像哈希证不了"pc 落在这份脚本的合法边界上"，这条检查不能删（P4） |
| 2 | 求值栈上溢或下溢（含 syscall 参数不足） |
| 3 | 指令预算耗尽（任务 1024/帧 或 全局 65536/帧，合成一个倒数，升序消耗先到先杀；引擎第二刀 §4.3） |
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
  ⚠️ **`wait(0)` 不算数**——它不让出，`loop { wait(0); }` 照样是 Fault(3) 死循环。
- **`WAIT` 的周期就是 n**（2026-08-01 语义修正，`ENGINE_VER` 11 → 12）：第 F 帧执行
  `wait(n)` ⇒ 第 F+n 帧接着跑。计数器存 `n−1`（yield 本身已吃掉当前帧）。旧实现存 `n`、
  周期是 `n+1`，凡自己记帧数的脚本一律偏 1/n——同一份镜像在新旧两版走出不同世界线，
  故旧回放/存档拒载。
- **`WAIT` 取栈顶低 16 位**：`wait(-1)` = 等 65535 帧、**`wait(65536)` 截断成 `wait(0)`
  ⇒ 真 no-op（不是"等 0 帧"），搁 `loop` 里即死循环**（截断语义，测试钉死）——帧数走大数
  请分段。
- **`KILL_CHILDREN` 按父槽号匹配，父死即断亲**（follow-ups C12⑤ 已修复，2026-07-20）：
  `TaskPool::kill` 释放一个槽时会顺手把所有指向它的存活子任务的 `parent` 清零——孤儿从
  父死那一刻起就是永久无父的独立任务，槽复用后新占用者的 `KILL_CHILDREN` 不会误杀前任
  的孤儿（本池无逐槽 generation，这是"父死立即断亲"代替代际戳的等价修法）。
- 难度（rank）= `globals` 约定槽（场景开局 `set_var` 写入），脚本 `get_var` 后自决——该槽
  （`GVAR_RANK=0`）落在**系统段**，脚本自己 `set_var(0, ..)` 会被 no-op 挡下（见"globals 段
  纪律"），想改 rank 得靠 game 层世界 API，不是脚本自己改自己的难度。值域 `0..=4`，判难度
  用注入的 `RANK_*` 常量（`>= RANK_HARD` 式比较），别写魔数。
- **`self_age` 是任务龄不是敌龄**：`SYS 32`（M1.5）量 `frame - task.born_frame`，次帧首跑时
  已经是 `1`（不是 `0`，出生当帧被 born_frame 门禁跳过、根本不执行）；对 `SPAWN` 出的子任务
  同理——子任务自己的 `born_frame` 是它的出生帧，不是父任务或 owner 实体的出生帧。
