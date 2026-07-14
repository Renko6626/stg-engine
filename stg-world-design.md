# stg-world 设计文档

**版本 v1.0** — 母文档 `design_doc.md`（v0.3）之下、对"底层世界"层级的细化设计。经 20 轮 grill 评审逐项拍板产出。
**范围**：stg-world = **ECL-VM 以外的裸世界本体**——实体池、运动、变换、碰撞、事件结算、快照/校验和、数学核，及其对上（ECL）与对外（表现层）的全部接口。
**方法论**：基本原则（Part I）→ 整体架构（Part II）→ 局部实现（Part III），附可扩展性（Part IV）、对母文档的修订清单（Part V）与遗留开放问题（Part VI）。
**与母文档的关系**：母文档是总纲，本文档是世界层的实施蓝图；两者冲突处以本文档为准，冲突点全部列于 Part V 待回写。

---

# Part I 基本原则

六条原则，编号 P1–P6。任何实现与后续修改不得违反；修改原则本身需过评审。

## P1 边界与形态：模块之实、crate 之纪

- stg-world 物理上是 **`stg-core` 内的一个模块**（`stg_core::world`），不拆独立 crate——world 与 VM 之间是每帧上万次调用的热路径，断层线的真正敌人（Godot/浮点/时钟）已被 `stg-core` 的 Cargo.toml 挡死，再拆 crate 无确定性收益、徒增接口摩擦。
- 但接口面**按 crate 级纪律书写**：ECL VM（以及一切调用方）**永不直接触碰池内存**，只走本文档定义的安全读/写 API。此纪律使"将来真要拆 crate"成为零成本机械动作。
- 依赖方向单向且世界无知：**ECL → world**。world 不 import 任何 ECL 类型，甚至不知道"任务"这个概念的存在。

## P2 step 所有权：组装层持宪法，世界出相位，导演槽制

- §3.5 的固定步进顺序（"确定性的命门"）**写在组装层**（`stg_core::step`）——唯一能同时看见 world 与 ECL 的地方。
- world 只暴露一组 `pub(crate)` **相位函数**（见 A4），各自可独立测试；顺序由组装层焊死。
- **PhaseGuard**（debug 构建）：`WorldBody` 内置相位状态机字段（`#[cfg(debug_assertions)]`，校验和跳过），每个相位函数入口断言前序相位正确，乱序即 panic——"顺序是宪法"从注释升级为运行期可执行的约定。
- **导演槽（director slot）**：step 3 形式化为一个槽位。规范绑定 = ECL 任务运行器；测试/基准/RL 场景绑定 = Rust 闭包（组装层提供 `step_with_director(world, input, director)` 入口，确定性责任归闭包作者，与 ECL 同一契约）；空任务池 = 零次循环 = 纯世界模拟，零特判。**ECL 只是导演槽的默认租户。**

## P3 单线程承诺：单 world 内永远单线程

- 一个 world 内的一切演化（积分、碰撞、结算）**永远单线程**，不留 rayon 口子。
- 理由：I4 的固定遍历序、事件收集序、RNG 消耗序在并行下逐 bit 保序需要处处特殊设计，而 8192 弹的整数运算量单核绰绰有余——为不存在的性能问题付永久复杂度税，不值。
- 并行的正确位置在 **world 之间**（RL 并行 env、CI 多回放并跑），零共享、零风险。
- 附带收益：`&mut World` 独占贯穿全程，借用、相位签名、快照时机全部简单。

## P4 错误策略三铁律 + 失败必须可诊断

| 类别 | 例子 | 行为 |
|---|---|---|
| **(a) 资源耗尽** | 池满、变换段池满、缓冲满 | **确定性降级，绝不 panic**：返回 `Handle::NULL` + 错误码，丢弃计入诊断计数器（随快照、**参与校验和**——两机必须丢得一样多，否则当场抓出） |
| **(b) 调用方契约违反** | 悬垂句柄、越界参数 | **确定性安全结果**：悬垂句柄返回"已失效"；坏参数使整条调用确定性 no-op + 计数。世界不信任任何调用方，所有入口全量检查 |
| **(c) 引擎自身 bug** | 相位乱序、复用槽未写满、碰撞阶段改状态 | **debug 构建帧内断言就地 panic**；release 构建不检查（零成本），靠 CI 的 debug 金向量挡在合入前 |

可诊断性机制（四件套）：

1. **Rust API 面**：所有写 API 返回 `Result<Handle, WorldStatus>`；`WorldStatus` 紧凑错误码枚举：`PoolFull(pool_id)` / `BadHandle` / `BadArgs(参数号)` / `Truncated`。
2. **ECL ABI 面**（值域 i32）：syscall 失败返回 `Handle::NULL`，同时 world 维护 **`last_status` 寄存器**（errno 模式；单线程 + 属于 World 状态 ⇒ 确定、可快照、参与校验和）。脚本可 `if last_status() == POOL_FULL` 做降级（"池满少发一圈"是合理的弹幕设计策略）。
3. **按类别累计的诊断计数器**（随快照、参与校验和）。
4. **返回值契约总表**（D12）：每个 API × 每种失败模式 × 返回什么 × 计入哪个计数器，一行一格写死。

`Handle::NULL`（`index == 0xFFFF`）是一等公民：所有返回句柄的 API 都可能给它，所有收句柄的 API 对它安全（视同已失效）。

## P5 无回调原则

**world 不持有任何回调、函数指针或外部注册表。** 一切"定制行为"只有两种合法形态：

1. **数据**：表 id、脚本 id（world 视为不透明 u16 存储与转发）；
2. **上层在相位之间读事件后自行行动**（如 step 9 的 ECL 事件挂钩相位）。

理由：函数指针进 World 直接违反 I7（memcpy 快照带不走、校验和无法哈希）；外部注册表意味着世界行为依赖一份不随快照的状态——回滚后行为漂移的完美温床。母文档 §3.3 的 `on_died` 回调构想据此**正式否决**。

## P6 全量校验原则

**住在 World 里的字段就参与校验和，无例外**——包括 `facing` 这类"纯表现"字段（"纯表现"的判断会随开发漂移，全量哈希杜绝"某字段悄悄变成逻辑相关后漏网"）。唯一例外是三条**纯输出缓冲**（`reqs` / `hits` / `frame_events`），它们不是状态、回滚重演时被确定性再生；每个 skip 必须在 derive 属性里给出理由字符串（宏强制）。

---

# Part II 整体架构

## A1 职责清单

| 进 | 内容 |
|---|---|
| ✅ | 定点数学核（Fx / Angle / 烘焙表 / CORDIC / isqrt；无状态纯函数库，**共享给 VM** 的算术指令使用） |
| ✅ | 池框架（`define_pool!` 宏：SoA、generation 句柄、free-list、存活掩码、Init 结构、校验和） |
| ✅ | 六个实体池（弹 / 敌 / 自机弹 / 道具 / Bomb 场 / 变换段）与自机状态 |
| ✅ | 运动积分、变换槽执行、碰撞、事件结算、清理各相位 |
| ✅ | RNG（PCG32 状态与原语；ECL 的 `rand()` 经读 API 消耗它） |
| ✅ | `hits` / `frame_events` / `reqs` 三条帧内缓冲 |
| ✅ | 快照（`copy_into`）+ 字段级校验和机制（`#[derive(Checksum)]`，crate 级工具、需覆盖 TaskPool） |
| ✅ | 世界公共写/读 API（`create_bullet` 等——ECL syscall 表的**世界侧实现**） |
| ✅ | 输入译码（`InputFrame` → 自机意图；`ActionInput` 类型定义归输入层） |
| ✅ | 通道 A/B 的数据结构与暴露形态（到"内存视图长什么样"为止） |
| ❌ | ECL VM、字节码、任务调度（ECL 层）；ActionInput 定义、回放格式（输入层）；网络、Godot 桥（各归其层） |

## A2 World 的拼装与状态所有权

`World` 结构体的**定义权在组装层**（stg-core 顶层）：

```rust
// stg-core 顶层（组装层）
#[repr(C)]
pub struct World {
    pub body:  WorldBody,   // stg-world 模块定义：世界本体的全部字段
    pub tasks: TaskPool,    // stg-ecl 模块定义：ECL 任务上下文
}
```

world 模块不 import ECL 类型（P1 依赖方向），但 `tasks` 物理上住在 `World` 里以满足 I7 的整块 memcpy 快照。快照/校验和 derive 作用于组装后的 `World` 整体。

```rust
// stg_core::world —— WorldBody（草案，字段细节见 Part III）
#[repr(C)]
pub struct WorldBody {
    frame: u32,
    rng: Pcg32,
    globals: [i32; 1024],              // 全局变量竞技场：纯 i32 槽，语义归脚本
    signals: [u32; 8],                 // 弹幕信号通道：每条存最后脉冲帧号（D4）
    players: [PlayerState; MAX_PLAYERS],
    boss_ui: [BossUiSlot; MAX_BOSSES], // 类型化公告板：脚本写、UI 读、世界自身不读
    bullets: BulletPool,               // cap 8192
    xforms:  XformSegPool,             // cap 2048 段 × 16 槽（特例手写，非宏生成）
    shots:   ShotPool,                 // cap 1024
    enemies: EnemyPool,                // cap 256
    items:   ItemPool,                 // cap 512
    bomb_fields: BombFieldPool,        // cap 16
    hits:         HitBuf,              // 碰撞命中缓冲（帧内私有）  #[checksum(skip)]
    frame_events: FrameEventBuf,       // 世界大事记（挂钩相位+上层读）#[checksum(skip)]
    reqs:         RenderReqBuf,        // 通道 B                    #[checksum(skip)]
    diag: DiagCounters,                // 诊断计数器（参与校验和！）
    last_status: u16,                  // errno 寄存器（参与校验和）
    #[cfg(debug_assertions)]
    phase_guard: u8,                   // PhaseGuard（校验和跳过：debug-only 时序护栏）
}
```

**`StageState` 概念删除**：原"关卡推进"就是关卡主控任务的 pc + locals（天然随 TaskPool 快照）；原"全局变量槽"即 `globals`；原"boss 阶段"即 `boss_ui` 公告板（当前符卡 id、血量刻度、倒计时——写入方是 ECL 的 `boss_set` syscall，读取方是断层线以上的 UI，**世界自身逻辑不读它**，超时/换卡判断都是 boss 主控任务的事）。

```rust
#[repr(C)]
pub struct BossUiSlot {
    enemy: Handle,        // 哪个敌人是 boss
    hp_ratio: Fx,         // 血条比例（脚本负责刷新）
    spell_id: u16,        // 当前符卡 id
    timer_frames: u16,    // 倒计时（脚本负责递减）
    phase_left: u8,       // 剩余阶段数（血条下的星星）
    active: u8,
}
```

## A3 静态数据：两类表、两条身份链

| | **引擎烘焙表** | **WorldTables** | **EclImage** |
|---|---|---|---|
| 内容 | sin/cos 表、easing 曲线表 | appearance 表、掉落表、图样描述符表、道具配置表、游戏配置段、角色配置表 | ECL 字节码 + 常量 |
| 物理形态 | 引擎二进制常量（`include_bytes!` commit 进仓库的原始字节） | 加载期只读数据 | 加载期只读数据 |
| 消费者 | world 数学核（VM 算术共享） | world 的 API（以 `&WorldTables` 参数传入） | 仅 ECL VM |
| 身份保障 | `engine_ver` + 烘焙表哈希（进握手/回放头，母文档 §2.1 纪律） | 与 EclImage **合并做内容哈希**，进回放头/联机握手 | 同左 |

WorldTables 清单（v1）：

- **appearance 表**：appearance_id → 默认判定半径 / sprite（`create_bullet` 查表拷入，显式参数可覆盖——母文档 §3.2.1 既定）；
- **掉落表**：drop_table_id → 掉落项列表（类型 × 数量 × 散布参数）；
- **图样描述符表**：pattern_id → 烘焙的发射参数块（appearance、count、speed、spread……），供变换 op `SPAWN_PATTERN` 以单个 id 引用（槽宽永不为胖参数膨胀，D4）；
- **道具配置表**：item_type → 分值（**v1 固定分值**，高度计价见 Part IV）/ 物理参数（弹出初速、终端速度、磁吸速度）/ 拾取半径；
- **游戏配置段**：场地半宽/高（逻辑 384×448）、越界回收边距、回收线（PoC）y 值、决死窗口 `DEATHBOMB_WINDOW = 8` 帧、复活参数（飞入时长、复活无敌帧）；
- **角色配置表**：character_id → 高速/低速移动速度、判定半径/擦弹半径、射击 CD、homing 转率等角色常量。

## A4 step 流水线 v2

组装层的宪法顺序（相位函数均为 `pub(crate)`，签名接收 `&mut WorldBody`（+ 所需只读参数），PhaseGuard 全程押运）：

| # | 相位 | 归属 | 内容 | 对母文档 §3.5 的变更 |
|---|---|---|---|---|
| 1 | `begin` | world | 清空 `reqs / hits / frame_events`（它们自上次 step 起为消费者存活至今）；PhaseGuard 复位 | 清空时点后移（原"帧末必空"作废，见 A6） |
| 2 | `decode_input` | world | `InputFrame` → `players[i].input`（译码后的动作位，存入 PlayerState：确定、随快照） | 不变 |
| 3 | **导演槽** | 组装层调 ECL | 规范租户 `ecl::run_tasks(&mut world.body, &mut world.tasks, &ecl_image, &tables)`：按任务池索引升序推进，owner 门禁 / born_frame 门禁 / 指令预算照母文档。syscall 经世界写 API **就地直接分配** | spawn_q 删除（A6） |
| 4 | `update_players` | world | 每自机：移动（input、低速、场界钳制）→ 生死状态机计时 → bomb 触发仲裁与 bomb 状态机 → **角色模块**静态分发（发弹、homing 转向、bomb 效果时间线）（A8） | 角色模块化（原为笼统"自机更新"） |
| 5 | `run_transforms` | world | 对有变换段的弹执行游标推进（相对 wait 制 + LOOP 护栏 + WAIT_SIGNAL + 插值 op，D4）。delay 期的弹跳过 | 语义升级（原"到期 canned op"） |
| 6 | `integrate` | world | 按池索引序：弹（双表示积分，D3）→ 自机弹 → 敌人（move_to 插值器优先，D5）→ 道具（重力/磁吸，D7）→ Bomb 场计时。`delay > 0` 的弹只递减 delay，不移动 | 双表示模型（D3） |
| 7 | `collide` | world | 按矩阵行序收集 `hits`（**只收集，不改状态**——硬规则不变）。delay 期与已清除标记的弹不参与 | 半径映射列（D8） |
| 8 | `settle` | world | **三趟结算**：清除/防护 → 伤害 → 计分/拾取（D9）。死亡掉落、`frame_events` 产出在此相位**直接分配/写入** | 掉落直接分配（A6） |
| 9 | **ECL 事件挂钩** | 组装层调 ECL | ECL 层扫描本帧 `frame_events`，对带 `death_script` 的 `EnemyDied` 派生任务（owner = 关卡句柄，死亡坐标经任务入参传递）（A7） | **新增相位** |
| 10 | `cleanup` | world | 越界（含边距）、寿命尽、已清除弹、死体回收入 free-list | "boss 阶段推进检查"移除（归 ECL） |
| 11 | `advance` | world | `frame += 1`。通道 A 视图与通道 B 请求对表现层可读 | 不变 |

## A5 事件系统：两条缓冲、两种消费者

| | `hits`（碰撞命中缓冲） | `frame_events`（世界大事记） |
|---|---|---|
| 记录 | `{ matrix_row: u8, active: u16, passive: u16 }`（6 B） | `{ kind: u8, a: Handle, x: Fx, y: Fx, data: [i32; 2] }`（~24 B） |
| 生产者 | 相位 7 收集循环（天然按矩阵行序×索引序，无需排序） | 相位 8 结算期产出的**已确认事实**：`EnemyDied{x,y,appearance,death_script}`、`PlayerDied`、`PlayerBombed`、`ItemPicked`…… |
| 消费者 | 相位 8 三趟过滤扫描。**帧内私有，永不暴露** | 相位 9 ECL 挂钩；表现层/上层经 `WorldView` 只读 |
| 容量 | 8192 | 512 |
| 生命周期 | 存活到下一帧 `begin` 清空 | 同左（否则相位 9 与表现层无物可读） |
| 快照/校验和 | 跳过（纯输出，重演确定性再生） | 同左 |

有效期契约：**events 仅在产出它的那次 step 之后、下次 step 之前有意义。**

耗尽语义：确定性丢弃 + 计数（P4）。但注意：丢弃一条 `PlayerHitByBullet` 意味着"本该死的自机活了"——确定性无损（两机同丢）而游戏性是错的，故 `hits` 容量按最坏情况给足（全弹同帧入擦圈），且 **debug 构建下 `hits` 溢出直接 panic** 而非静默计数。

## A6 创建即分配（spawn_q 删除）

母文档的 `spawn_q` 存在两处自相矛盾（结算期掉落 vs "帧末必空"；`create_bullet -> BulletHandle` 的句柄从未分配的槽位何来），**正式移除**。替代语义：

- 实体从 `create_*` 调用那一刻起存在：句柄立即真实有效，字段经宏生成的 `Init` 结构体**一次写满**（写满槽从纪律升级为编译期事实，D2）；
- **每个相位处理"它运行时活着的实体"**——相位 3/4 创建的弹当帧走变换/积分/碰撞（与母文档"当帧生成当帧参与"一致）；相位 8 结算掉落的道具当帧只过 cleanup 检查，次帧首次移动。规则统一、无特例；
- 单线程 + 固定调用序 ⇒ free-list 分配序完全确定，确定性零损失。

## A7 死亡结算机制

- **机械部分（world 固定逻辑、数据驱动）**：相位 8 趟二伤害结算中 hp≤0 ⇒ 标记死亡、按 `drop_table` 查 WorldTables 直接分配掉落（散布消耗世界 RNG，消耗序 = 结算序，确定）、发死亡特效请求（`reqs`）、产出 `EnemyDied{x, y, appearance, death_script}` 入 `frame_events`。槽位相位 10 回收。
- **脚本部分（经相位 9 挂钩）**：ECL 层对带死亡脚本的 `EnemyDied` 派生任务——owner = **关卡句柄**（不能是垂死敌人，否则次帧 owner 门禁杀之），死亡坐标经任务入参传入（带参派生机制归 ECL 文档；**事件带哪些字段归本文档**）。死亡逻辑就是一段普通 ECL：同语言、同 syscall、同编译管线，作者体验 = "敌人定义 = 主控 sub + 可选死亡 sub"。
- 分工：**例行掉落走掉落表**（快、零脚本成本），**异行为走死亡脚本**（告别弹、演出、额外掉落），共存不互斥。
- 敌人死亡**默认不清弹**（既定语义：其弹上任务因 owner 失效而死，弹本体存续）。

## A8 自机架构：输入信号驱动的确定性状态机

自机与 ECL 无关（ECL 是 **Enemy** Control Language——真东方的自机也全部硬编码在 exe）。自机 = **一台由输入信号驱动的确定性状态机**，全部住在世界侧，按**角色模块**组织：

- `PlayerState.character_id` 在相位 4 **静态分发**（`match`，编译进引擎的角色模块——不是函数指针，零 P5/I7 冲突）到各角色的 `update_shot / update_bomb / steer_shots` Rust 函数；
- **世界管"身体与账本"**（角色无关的公共骨架）：移动积分、低速切换、场界钳制、中弹判定、决死窗口状态机、死亡/复活/无敌计时、bomb 触发仲裁（查库存、消库存、**触发帧立即无敌**）、残机/bomb/power/graze/score 账本；
- **角色模块管"火力与个性"**：发弹模式（读 `players[i].input` 动作位）、homing 弹转向（逐帧扫最近敌人，转率为角色常量）、bomb 效果时间线（`bomb_phase/bomb_timer` 小状态机驱动，铺 BombField 实体、发演出请求）；
- 多帧演出用 `PlayerState` 内的纯数据状态机字段驱动，随快照、参与校验和；
- 跨层备注（输入层）：`ActionInput.buttons` 现为 u8，基础动作已占 7 位；**建议扩为 u16**，世界侧按位号消费、不关心按位语义——为将来"自机技能 A"这类扩展动作留空间。

## A9 通道 A/B 的世界侧 API 形态

**两组 API 物理分离**，防"表现层顺手改世界"：

| | ECL/导演写读 API | 表现层视图 API |
|---|---|---|
| 签名 | `&mut WorldBody`（+ `&WorldTables`） | `&WorldBody` |
| 失败 | `Result<_, WorldStatus>`（P4 契约） | 永不失败 |
| 成员 | `create_bullet / create_bullets_batch / create_player_shot / spawn_enemy / drop_item / set_vel / set_speed / set_angle / aim_player / move_to / set_var / get_var / boss_set / pulse_signal / emit_req / rand 族 / last_status / nearest_enemy / player_x/y / enemy_hp / frame` | `view() -> WorldView` / `take_requests() -> &[RenderReq]` / `frame_events() -> &[FrameEvent]` |

**通道 A —— 零拷贝类型化视图**：

```rust
pub struct WorldView<'w> {
    pub frame: u32,
    pub players: &'w [PlayerState],
    pub boss_ui: &'w [BossUiSlot],
    pub bullets: PoolView<'w, BulletCols>,   // SoA 各字段裸切片 + 存活位字 &[u64] + 活跃计数
    pub enemies: PoolView<'w, EnemyCols>,
    pub shots:   PoolView<'w, ShotCols>,
    pub items:   PoolView<'w, ItemCols>,
    pub events:  &'w [FrameEvent],
}
```

- **死槽照常暴露，过滤是消费者的义务**：视图提供 `iter_alive()` 便利迭代器，但 MultiMesh 桥、numpy 观测这类批量消费者直接拿裸切片 + 位掩码自己扫，吃满 SoA 带宽红利；
- 定点→浮点转换**只发生在消费者一侧**（母文档 §6.4）；
- 有效期由借用检查器天然保证：`WorldView` 存活期间无法 step；
- **PoolView 布局半冻结**：字段顺序/对齐是跨语言契约（PyO3 numpy 零拷贝），改动需过评审 + bump `engine_ver`。

**通道 B —— drain 语义**：`take_requests()` 读走本帧请求切片；缓冲在下帧 `begin` 清空，headless 下无人消费 = 零成本；帧内多次调用返回同一切片（幂等，reqs 不是状态）。

---

# Part III 局部实现

## D1 数学核

**newtype 强类型**（拍板）：

```rust
#[repr(transparent)] pub struct Fx(i32);      // Q16.16
#[repr(transparent)] pub struct Angle(u16);   // BAM，一圈 = 65536
```

- 动机：定点最经典的静默灾难是"忘了移位的裸乘法"（`a * b` 差 65536 倍还编译通过）。newtype 下 `Fx × Fx` 只能走重载 `mul`（i64 中转 + 移位），此 bug 类从 code review 祈祷变成编译错误。`Angle` 的回绕加减、`Angle ± i16` 增量同理受控。
- `repr(transparent)` ⇒ ABI/布局与裸 i32/u16 全同：SoA、memcpy 快照、PyO3 零拷贝无感。边界处（ECL ABI、通道 A）显式拆包。

**运算规格（契约的一部分，处处相同重于对错）**：

| 原语 | 规格 |
|---|---|
| `mul(a, b)` | `((a as i64 * b as i64) >> 16) as i32`——算术右移，向负无穷截断 |
| `div(a, b)` | `(((a as i64) << 16) / b as i64) as i32`——向零截断 |
| 溢出 | debug panic；release wrapping + 帧内断言计数 |
| `sin / cos / sincos` | 四分之一波对称表 **16384 × i32**（64 KB），入 `Angle` 出 `Fx`；取整规则 round-half-to-even |
| `atan2(y, x) -> Angle` | 整数 CORDIC，**迭代次数钉死 16 轮**（次数是契约；改 = bump engine_ver） |
| `isqrt(u64) -> u32` | 整数牛顿法，单调收敛、终止条件写死 |
| easing 表 | 每曲线 **257 × i32**（Q16.16 归一化 [0,1]），首版烘 8 条：linear（**M0-1 实现改为亦烘入表**：统一表结构、`ease()` 无特例，代价 +1KB）、quad/cubic 的 in/out/in-out、smoothstep。各条公式/手感见 `math::easing::Easing` 变体注释 |

全部烘焙表遵守母文档 §2.1 纪律：**生成一次、commit 原始字节、`include_bytes!` 嵌入；CI 再生成并断言与 commit 字节逐位相同**；表哈希进握手/回放头。

**定点乘法规范（运算基线，M0-1 补充）**：`Q16.16 × Q16.16` 的 raw 积天然是 **Q32.32**（小数位 16+16=32）。`Fx::mul` = 得 Q32.32（i64 中间量）后 **`>>16` 归一化回 Q16.16（i32）**——这步既丢低 16 位、又是溢出的来源：结果须 ≤ ±32768，故**仅当至少一个操作数 ≤ ~1.0 时安全**（`speed × 单位向量`、`位移 × easing-t`、`坐标 × 归一化权重`）。**平方距离 / 模平方 / 点积等"离开屏幕空间"的量：保留 Q32.32 于 i64、不归一化**（`math::geom::len_sq`），既不溢出又不丢精度；碰撞比较时 `r²`（`r.raw()²`）同为 Q32.32，直接比、不开根（D8）。通用律：`Q(m).f × Q(m).f = Q(2m).(2f)`，`>>f` 才归一化回 `Q(m).f`；**保留双宽 = 累加器模式**。两个大坐标相乘却 `>>16` 塞回 `Fx` = 经典溢出翻车。

## D2 池框架：`define_pool!` 宏

**一次声明、处处一致**——池纪律的每一条都是"加字段时容易忘"型风险，唯一可靠解法是单一真相源自动生成（**M0-3 落地为 stg-derive 的函数式 proc-macro**；下列产出经 grill 评审有若干偏离，已就地标注）：

```rust
define_pool! {
    Bullet, cap = 8192,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        // ... 见 D3
    }
}
```

宏展开产出：

1. **SoA 数组**（每字段一条 `[T; CAP]`，数组内无 padding，字段级校验和天然跳过字段间 padding）；
2. **句柄类型**：`{ index: u16, generation: u16 }` 打包 u32；**每次 alloc `generation+1`**（活槽 gen≥1，杀零句柄；M0-3 细化原"复用时+1"）；`valid = alive && gen 匹配`；`NULL = index 0xFFFF`；access → `Option`；
3. **分配/释放**：**存活位掩码即分配器**（`[u64; cap.div_ceil(64)]`）——alloc = 最低空位（末字按 `cap%64` 掩码防幽灵位），free = 清 bit；**无独立 free-list**（M0-3 偏离原 LIFO：掩码兼分配、少一个状态/desync 面，确定性不变）；升序遍历；
4. **`XxxInit` 全字段初始化结构体**：分配必须整个传入——**"写满槽"是编译期事实**（漏字段编译不过）。二线防御改为**宏全覆写单测**（原运行时"遗留值"断言对合法同值写入会误报，M0-3 弃用）；
5. **迭代器**（升序 alive 遍历）；**PoolView（通道 A）推迟 M2**（唯一消费者 Godot/PyO3，随其真实布局定"半冻结契约"）；
6. **Checksum 实现**（并入 derive 体系，D11）。

**新增池的标准动作**（未来激光池等）：写一个 `define_pool!` 声明 → 在 `WorldBody` 加字段 → 在相应相位函数加处理循环 → 容量进 D10 预算表 → 碰撞矩阵若新增行则过评审。五步，无隐藏步骤。

**特例**：`XformSegPool` 不套宏、单独手写——它是"整段借出、随弹释放"的段式分配（无逐槽 generation），模式不同（D4）。

## D3 弹池：双表示运动模型

**模型拍板（经 TH16 逆向实证修正）**：ZUN 式双表示——

- **积分真相 = `vx/vy`**：相位 6 主路径就是 `x += vx; y += vy`。直进弹（绝大多数）每帧两次加法、零查表；
- **`speed/angle` = 作者视图缓存**：双向同步责任**全部封装在 setter 内**——脚本与变换 op 永远摸不到裸字段（P1 纪律），"忘了回填"的经典火药桶从"每个弹幕作者"收缩为"引擎内一个封闭 setter 集合"，一次写对 + 单测覆盖：
  - 极坐标 setter（`set_speed/set_angle/turn/aim_player`）→ 改后 `(vx,vy) = polar2vec(speed, angle)`（一次 sincos）；
  - 笛卡尔 setter（`set_vel`）→ 改后 CORDIC atan2 + isqrt 回填极坐标，**带速度阈值**（阈值规则钉死进契约，低速不回填防抖）；
- **连续效果 = 模式位 + 字段**（变换 op 只负责在排程点把它们打开/改参，两机制正交）：
  - `POLAR_FX`：`angle += ang_vel; speed += accel;` 后刷 vx/vy（仅此类弹付查表税）；
  - `CART_FX`：`vx += ax; vy += ay;` 后按阈值回填极坐标（sprite 朝向不失真）；
  - **互斥：置一清另一**（即 TH16 `c68 &= ~0x9` 的语义）。

**字段清单（定稿）**——SoA，~54 B/弹，8192 弹 ≈ 450 KB：

| 字段 | 类型 | 说明 |
|---|---|---|
| `x, y` | Fx×2 | 位置 |
| `vx, vy` | Fx×2 | **积分真相** |
| `speed` | Fx | 作者视图（缓存） |
| `angle` | Angle | 作者视图（缓存），兼 sprite 朝向 |
| `ang_vel` | i16 | BAM/帧（POLAR_FX） |
| `accel` | Fx | 沿向加速（POLAR_FX） |
| `ax, ay` | Fx×2 | 笛卡尔加速（CART_FX，重力/漂移） |
| `sprite` | u16 | 外观（appearance 展开） |
| `radius` | Fx | 判定半径（appearance 默认，可覆盖） |
| `delay` | u8 | 出现延迟帧：ZUN"发光膨胀预告"期——**不移动、不判定、变换不走、只递减 delay** |
| `life` | u16 | 寿命帧；`0xFFFF` = 无限 |
| `flags` | u8 | POLAR_FX / CART_FX / 已清除 / 反弹计数(2位) / 保留 |
| `grazed_by` | u8 | 每自机一位的擦弹掩码（逐弹一次语义） |
| `transform_head` | u16 | 变换段索引；`0xFFFF` = 哑弹 |
| `xform_wait` | u16 | 变换游标倒计时（D4） |
| `xform_next` | u8 | 下一未发槽位 |

母文档草案的 `layer: u8` **删除**：池即层（弹池整池 = EnemyBullet 层，碰撞矩阵按池配对）。`ax/ay` 的加入使"重力抛物线弹"原生化，不再需要退化成任务弹。

## D4 变换系统

**设计基线（TH16 逆向的取舍结论）**：ZUN 的弹是 5.2 KB 的微型 VM，因为他没有任务弹这一档；我们有三档制，档二只收"**全屏级**"能力（几百颗弹同时要的），少数派行为归档三（任务弹）。据此**采纳**：wait 门相对时序、计数循环、信号触发（EX_REACT 对应物）、限时插值（EX_STEP 对应物）、弹生弹（瘦身为表引用）；**否决**：逐效果私有状态块（8192 × 5 KB = 40 MB，快照预算爆炸）、EX_SAVE、任意条件逻辑（档三伺候）。ZUN 的"效果并发"在我们这里等价成立：连续效果是**持久字段**，序列游标只负责排程——"之字 + 加速" = 循环里 TURN + 开局一次 SET_ACCEL，单游标天然并发。

**段池**：`XformSegPool = [XformSlot; 2048 段 × 16 槽]`（384 KB，手写特例）。`create_bullet` 把序列**拷贝**进弹自有段（不共享、无引用计数），随弹回收。段池满 ⇒ `create_bullet` **整体失败**（`NULL + PoolFull`）——不做"退化成哑弹"的部分成功：弹出来了却不拐弯是最难查的静默错误，宁缺一颗弹（两者皆确定，选可诊断的）。

**槽格式（12 B，相对 wait 制）**：

```rust
#[repr(C)]
struct XformSlot { wait: u16, op: u8, _pad: u8, args: [i32; 2] }
```

语义：**发射本 op 后，等 `wait` 帧再执行下一槽**（wait=0 同帧连发）。args 无类型——I1 之下 Fx 与 i32 同位宽，"整数槽 vs 浮点槽"的 ZUN 区分蒸发，每个 op 自己规定参数语义。

**胖参数三层拳**：① 常规 op 两参封顶；② **声明元数的多槽 op**——引擎内静态 `ARITY[op]` 表（0 或 1 个扩展槽），游标步进 = `1 + ARITY[op]`，扩展槽的 args 兼作该 op 的 scratch；③ 真正的大参数块走 WorldTables 图样描述符表间接引用（`SPAWN_PATTERN(pattern_id)`）。

**段即 scratch 原则**：弹拥有自己的段拷贝，故 op 把运行时状态就地写回自有段是合法的（随弹快照、确定性无损）——`LOOP` 的剩余计数、`STEP_*` 的插值起点/已历帧都存在这里，弹本体零新增字段。

**op 清单定稿（17 个）**：

| 类 | op | args | 槽数 | 语义 |
|---|---|---|---|---|
| 瞬时 | `SET_SPEED` / `ADD_SPEED` | speed | 1 | 改速率，回填 v |
| | `SET_ANGLE` / `TURN` | angle | 1 | 改向，回填 v |
| | `AIM_PLAYER` | Δangle | 1 | 瞄准最近自机+偏移，回填 |
| | `SET_SPRITE` | id | 1 | 换贴图 |
| | `SET_LIFE` | n | 1 | 重设寿命（含"到时自爆"用法） |
| | `SPAWN_PATTERN` | pattern_id, Δangle | 1 | 按描述符表发一批子弹（烟花弹） |
| 连续开关 | `SET_ANG_VEL` | ω | 1 | 开 POLAR_FX |
| | `SET_ACCEL` | a | 1 | 沿向加速，开 POLAR_FX |
| | `SET_GRAVITY` | ax, ay | 1 | 开 CART_FX（清 POLAR_FX） |
| | `STOP_FX` | — | 1 | 清全部模式位 |
| | `BOUNCE_ARM` | n≤3, walls | 1 | 反弹待命（flags 2 位计数） |
| 插值 | `STEP_SPEED` | target, frames\|easing | **2** | 限时缓动到目标速率（scratch 在扩展槽） |
| | `STEP_ANGLE` | target, frames\|easing | **2** | 限时缓动到目标角 |
| 控制 | `LOOP` | target_slot, count | 1 | 游标跳回；count 就地递减，0 = 无限 |
| | `WAIT_SIGNAL` | ch | 1 | 停在此 op，`signals[ch] == 当前帧` 才放行 |

**执行算法（相位 5，每有段的活弹）**：

```
if delay > 0 → 跳过（激活前变换不走）
if xform_wait > 0 { xform_wait -= 1; return }
loop {
    if xform_next 越界 或 op == END → 序列终结，return
    if op == WAIT_SIGNAL 且 signals[ch] != frame → return   // 停驻等待
    发射 op（可能改字段/开模式位/写自有段 scratch）
    if op == LOOP → 游标跳转; return                        // LOOP 护栏：本帧到此为止，防帧内自环死循环
    xform_wait = slot.wait; xform_next += 1 + ARITY[op]
    if xform_wait > 0 → return                              // wait 门
}
```

**信号通道（EX_REACT 对应物）**：`WorldBody.signals: [u32; 8]`，每条存最后脉冲帧号；ECL syscall `pulse_signal(ch)`（相位 3 写入）；`WAIT_SIGNAL` 相位 5 消费。**边沿触发**：只有正停驻在该 op 上的弹响应——"全场弹听号令齐转向"的符卡语义，零额外状态。

**ECL ABI 衔接**（既定丙方案的落地形态）：locals 区间引用 `(xform_off, xform_cnt)`；每槽占 3 个 locals 字——`word0 = (wait << 16) | (op << 8)`，`word1/2 = args`。16 槽 = 48 字 ≤ 64（Task.locals 容量自洽）。帧内断言守 `xform_off + xform_cnt×3 ≤ 64`。

## D5 敌人池

SoA，~64 B/敌，256 敌 ≈ 16 KB：

| 组 | 字段 | 说明 |
|---|---|---|
| 运动 | `x y vx vy: Fx×4` | 与弹同构（笛卡尔执行面） |
| 移动插值器 | `mv_from_x/y, mv_to_x/y: Fx×4, mv_t: u16, mv_dur: u16, mv_easing: u8, mv_active: u8` | `move_to(t, x, y, easing)` 的世界侧状态机 |
| 生命 | `hp: i32, hp_max: i32` | max 供 boss_ui 血条比例 |
| 判定 | `radius: Fx`（体碰）, `hurtbox: Fx`（受击） | **双半径**，映射见 D8 |
| 状态 | `invuln: u16, hit_flash: u8, flags: u8` | 无敌帧、受击闪计时、位标记（dying 预留位） |
| 外观 | `sprite: u16, anm_state: u16` | 展示 id；anm_state 供表现层选动画，**世界不解释** |
| 挂钩 | `main_task: Handle, death_script: u16, drop_table: u16` | 主控任务、死亡脚本（不透明转发）、掉落表 |
| 计分 | `score: u16` | 击破基础分 |

**`move_to` 语义（拍板）**：插值器激活期间**完全接管位置**（`pos = from + (to−from)·ease(t/dur)`），vx/vy 冻结不积分；到期 `mv_active = 0` 且 **vx/vy 清零**——到点即悬停（ZUN boss 移动语义）。easing 查 D1 烘焙表。

**双半径动机**：真东方"自机贴着 boss 擦而不撞死、自机弹却打得中"依赖受击圈（大）≠ 体碰圈（小）的区分；本质是同一实体在**不同碰撞矩阵行**中扮演不同大小的角色（D8 的半径映射列）。

## D6 自机

`PlayerState`（MAX_PLAYERS = 2，~80 B/人）：

| 组 | 字段 |
|---|---|
| 身体 | `x y: Fx×2, character_id: u8, facing: i8`（facing 纯表现字段，**照样参与校验和**——P6） |
| 判定 | `hit_radius: Fx, graze_radius: Fx`（角色配置拷入；graze 圈兼道具拾取圈） |
| 输入 | `input: u16`（相位 2 译码写入的动作位） |
| 生死状态机 | `life_state: u8, state_timer: u16, invuln: u16` |
| bomb 状态机 | `bomb_phase: u8, bomb_timer: u16` |
| 火力 | `shot_cd: u8, power: u16`（定点百分制 0–400 = 0.00–4.00） |
| 账本 | `lives: u8, bombs: u8, life_pieces: u8, bomb_pieces: u8, score: u64, graze: u32`（**score u64**：东方真实分数上千亿，u32 溢出） |

**生死状态机**（全整数帧）：

```
Alive ──中弹(趟二)──► DeathWindow（决死窗口, DEATHBOMB_WINDOW=8 帧, 引擎常量/游戏配置段）
  ▲                        │
  │  bomb 输入且 bombs>0    │ 窗口耗尽
  │ （消 bomb, 立即无敌）    ▼
  ├──────────────────  Dead：lives−1、掉 power、PlayerDied 事件、respawn_timer
  │                        │
  └── invuln 耗尽 ◄── Respawning（场底飞入, 无敌）
```

- **死亡连带结算世界侧固定**（掉 power、power 道具回撒规则）：它是账本公平性的一部分，与中弹判定同级，不容每个关卡脚本重写；ECL 只收 `PlayerDied` 事件做演出。
- **bomb 触发仲裁世界侧**（触发帧立即无敌——决死救人的帧精确性不依赖任何脚本/模块延迟）；bomb **效果**由角色模块经 `bomb_phase/bomb_timer` 状态机逐帧驱动（铺 BombField、发演出请求），晚一帧铺开在演出上不可见。

**BombFieldPool**（bomb 判定场实体化，碰撞矩阵 6/7 行的主动方）：cap 16，字段 `x y: Fx, radius: Fx, dmg_per_frame: u16, owner: u8, life: u16`。角色模块创建，寿命尽自灭。

## D7 自机弹池与道具池

**自机弹池**（cap 1024，~28 B）：`x y vx vy: Fx×4, damage: u16, radius: Fx, sprite: u16, owner: u8, flags: u8`。

- **不给变换槽**：自机弹行为是可枚举的少数几种（直线/追踪/曲线），全部由角色模块驱动——homing = `flags.HOMING` 位 + 角色模块相位 4 逐帧转向（转率角色常量，`nearest_enemy` 世界查询助手）。将来若需排程序列，宏生成池加字段是机械动作（Part IV）。

**道具池**（cap 512，~22 B）：`x y vx vy: Fx×4, item_type: u8, magnet_to: u8（0xFF=无）, timer: u16（闪烁/消失倒计时）`。

行为世界侧固定（账本公平性同级）：

```
弹出（初速向上，散布消耗世界 RNG）→ 重力减速 → 终端速度匀速下落
  → 自机越过回收线(PoC) 或 进入拾取圈 → 磁吸至自机（magnet_to 锁定）→ 拾取结算
```

计价 **v1 = 每类型固定分值**（WorldTables 道具配置表）；高度计价机制待考证后升级，升级点局限在"结算趟三读价值"单个函数内（Part IV）。

**坐标系约定（全局）**：ZUN 式**中轴原点**——x ∈ [−half_w, +half_w]（场地中线 = 0），y ∈ [0, height] 自顶向下；逻辑场地 384×448，**1 Fx 整数位 = 1 逻辑像素**。对称弹幕（东方绝对主流）天然镜像（angle 取负即可）。场地尺寸/越界边距/回收线全在游戏配置段。

## D8 碰撞

**矩阵 v2（带半径映射列）**——同一实体在不同行用不同半径，映射是矩阵表的一列而非散在代码里：

| # | 主动 | 被动 | 主动半径 | 被动半径 | 事件 |
|---|---|---|---|---|---|
| 1 | EnemyBullet | PlayerHit | bullet.radius | player.hit_radius | PlayerHitByBullet |
| 2 | EnemyBullet | PlayerGraze | bullet.radius | player.graze_radius | Graze |
| 3 | EnemyBody | PlayerHit | enemy.**radius**（体碰） | player.hit_radius | PlayerHitByBody |
| 4 | PlayerShot | EnemyBody | shot.radius | enemy.**hurtbox**（受击） | EnemyDamaged |
| 5 | Item | PlayerGraze | item 拾取半径（配置表） | player.graze_radius | ItemPicked |
| 6 | BombField | EnemyBullet | field.radius | bullet.radius | BulletCleared |
| 7 | BombField | EnemyBody | field.radius | enemy.**hurtbox** | EnemyDamaged |

- 全圆判定、i64 平方距离比较、不开根（I1）；
- `delay > 0` 与已清除标记的弹**不参与检测**；无敌帧敌人跳过 4/7 行伤害（但事件照收、结算时判）；
- **只收集不改状态**硬规则不变（改状态会让前面的碰撞结果影响后面的判定）；
- O(N×M) 直扫（敌弹×自机 = N×1，其余两侧皆小）；**broadphase 接口预留**：碰撞相位输入 = 各池位置切片、输出 = `hits`，未来加均匀网格只替换相位内部实现，接口零变化。

## D9 结算三趟（相位 8）

对 `hits` 按类别三趟过滤扫描（每趟内保持收集序，全程确定）：

1. **趟一 · 清除/防护**：BombField 清弹（行 6）——被清弹打"已清除"标记 + `BulletCleared`。**先于中弹**，同帧 bomb 能救下本会命中的弹；
2. **趟二 · 伤害**：行 4/7 扣血（无敌帧过滤在此判）→ hp≤0 走 A7 死亡结算（标记、掉落直接分配、特效请求、`EnemyDied` 事件）；行 1/3 自机中弹（**跳过已清除的弹**）→ 生死状态机转移（Alive → DeathWindow）；
3. **趟三 · 计分/拾取**：Graze（行 2，`grazed_by` 位掩码逐弹一次，独立于中弹）；ItemPicked（行 5）→ 按道具配置表入账本，power/残机蜡/bomb 蜡的进位规则世界侧固定。

## D10 容量常数与内存预算

全部为引擎常量；初值如下，**金向量实测后调参**（母文档 §10.4 精神不变）：

| 项 | 容量 | 单价 | 小计 |
|---|---|---|---|
| 弹池 | 8192 | ~54 B | ~450 KB |
| 变换段池 | 2048 段 × 16 槽 | 12 B/槽 | 384 KB |
| 自机弹池 | 1024 | ~28 B | 29 KB |
| 敌人池 | 256 | ~64 B | 16 KB |
| 道具池 | 512 | ~22 B | 11 KB |
| Bomb 场池 | 16 | ~16 B | <1 KB |
| 任务池（ECL 类型，住组装层 World） | 512 | ~600 B | 307 KB |
| globals | 1024 × i32 | | 4 KB |
| hits | 8192 × 6 B | | 48 KB |
| frame_events | 512 × 24 B | | 12 KB |
| reqs | 256 × 28 B | | 7 KB |
| 自机×2 / boss_ui×2 / signals×8 / RNG / 帧计数 / 诊断计数器 | | | <1 KB |
| **World 总计** | | | **≈ 1.3 MB** |

16 帧快照环 ≈ 21 MB（环归回滚调度器所有，非 stg-world 财产）。

## D11 快照与校验和

- **快照 API 的全部 = `fn copy_into(&self, dst: &mut World)`**：World 是 POD（I7），一次 ~1.3 MB memcpy、零分配。环形缓冲、保留策略、回滚重演全部归消费者（M3 harness / phase 2 net）；stg-world 只承诺"World 可整块复制"这一条性质。
- **哈希算法 vendored 钉死**：**FNV-1a 64**，五行实现 commit 进仓库——校验和算法是确定性契约的一部分，**绝不走外部依赖**（crate 升级悄悄改实现 = 全体回放与金向量作废）。若实测成为 CI 瓶颈，换 vendored xxHash64，换算法 = bump `engine_ver`。
- **`#[derive(Checksum)]` 两种输出**：
  - `checksum() -> u64`：按字段声明序合并——联机随包 / CI 断言用；
  - `checksum_report()`（debug）：**逐字段哈希清单** `[(字段路径, u64)]`——desync 时 CI 自动改跑 report 模式，直接定位"`bullets.vx` 在第 N 帧分歧"。这是字段级方案相对整块哈希的杀手级红利；
  - `#[checksum(skip = "理由")]`：跳过必须给理由字符串（宏强制），现有 skip 仅 `reqs / hits / frame_events`（纯输出）与 `phase_guard`（debug-only 护栏）；
  - 防漏：字段自动纳入（单一真相源）；CI 加"World 尺寸/字段数变更即红"守卫。
- **字节序契约**：支持平台限定**小端**（x86_64 / aarch64 全小端），derive 按 SoA 数组原始字节哈希；大端平台不在支持矩阵，不为其付 per-element 转换税。
- SoA 数组内部无 padding（同类型连续），整条哈希；字段间 padding 被字段级遍历天然跳过——`repr(C)` padding 垃圾字节的假 desync 问题就此根除（母文档 §3.1 陷阱的解）。

## D12 返回值契约总表

| API | 失败模式 | 返回 | last_status | 计数器 |
|---|---|---|---|---|
| `create_bullet` | 弹池满 | `NULL` | `POOL_FULL(BULLET)` | `diag.pool_full[BULLET]` |
| | 变换段池满 | `NULL`（**整体失败**，不产弹） | `POOL_FULL(XFORM)` | `diag.pool_full[XFORM]` |
| | xform 区间越界 locals | `NULL` | `BAD_ARGS(xform)` | `diag.contract_viol` |
| | appearance 越表 | `NULL` | `BAD_ARGS(appearance)` | `diag.contract_viol` |
| `create_bullets_batch` | 中途池满 | 已成部分保留，返回成功数 | `POOL_FULL(BULLET)` | `diag.pool_full[BULLET]` |
| `create_player_shot` | 池满 | `NULL` | `POOL_FULL(SHOT)` | `diag.pool_full[SHOT]` |
| `spawn_enemy` | 池满 | `NULL` | `POOL_FULL(ENEMY)` | `diag.pool_full[ENEMY]` |
| `drop_item` | 池满 | `NULL` | `POOL_FULL(ITEM)` | `diag.pool_full[ITEM]` |
| `set_vel / set_speed / …`（收句柄类） | 句柄悬垂/NULL | no-op | `BAD_HANDLE` | `diag.contract_viol` |
| `move_to` | 句柄悬垂 / dur=0 | no-op | `BAD_HANDLE / BAD_ARGS` | `diag.contract_viol` |
| `set_var / get_var` | 槽号 ≥ 1024 | no-op / 返回 0 | `BAD_ARGS` | `diag.contract_viol` |
| `boss_set` | 槽号 ≥ MAX_BOSSES | no-op | `BAD_ARGS` | `diag.contract_viol` |
| `pulse_signal` | ch ≥ 8 | no-op | `BAD_ARGS` | `diag.contract_viol` |
| `emit_req` | reqs 满 | 丢弃 | `TRUNCATED` | `diag.reqs_dropped` |
| （内部）hits 满 | — | 丢弃（**debug panic**） | — | `diag.hits_dropped` |
| （内部）frame_events 满 | — | 丢弃 | — | `diag.events_dropped` |

注：`create_bullet` 的 `task_script` 参数**不属于世界 API**——世界不认识任务。ECL syscall 绑定层自行组合：先调 `world.create_bullet(...)` 拿句柄，再调 `ecl::spawn_task(script, owner = 句柄)`。这是 P1/P2 边界的直接推论。

---

# Part IV 可扩展性（预留后路清单）

每条都设计为**纯增量**，不推翻本文档任何决定：

1. **激光池**：走 D2"新增池标准动作"五步；判定形状（线段/胶囊）需扩碰撞矩阵一行 + 一个新距离原语（平方点线距，仍不开根）；
2. **ZUN 式 ECL 中断 / 垂死状态**（死亡机制乙方案）：世界侧只需追加敌人 `interrupt_flags` 字段 + dying 状态位（已预留），VM 侧机制归 ECL 文档；
3. **脚本化自机**：加"自机 ECL 任务"纯增量挂上（owner = 自机），世界侧账本/仲裁不动；
4. **道具高度计价**：升级点局限在结算趟三的价值函数一处 + WorldTables 加一条价值曲线；
5. **碰撞 broadphase 均匀网格**：相位内部实现替换，接口（位置切片进、hits 出）零变化；
6. **新触发条件 op**（`WAIT_NEAR_PLAYER` 等）：op 空间 u8 余量充足，逐条评审加入；
7. **自机弹变换槽**：宏生成池加三字段（`transform_head/xform_wait/xform_next`），相位 5 循环推广；
8. **`spawn_task_now`**（worklist drain）：ECL 层事项，世界无涉（母文档 §4.3 既定）；
9. **Q32.32 位置升级**：若金向量实测出精度病例，`Fx` newtype 使受影响面收敛在数学核与池字段类型（母文档 §10.2 未决项）。

---

# Part V 对母文档（design_doc.md v0.3）的修订清单

回写时逐条核对：

1. **§3.1**：`spawn_q` 字段删除（A6）；`events` 拆为 `hits` + `frame_events`，生命周期从"帧末必空"改为"存活至下帧 begin"（A5）；`stage: StageState` 替换为 `globals: [i32; 1024]` + `boss_ui`（A2）；新增 `signals / diag / last_status / bomb_fields`；
2. **§3.2**：弹的运动模型改双表示（D3）；字段清单以 D3 为准（`layer` 删除、`delay/ax/ay` 等加入）；`transform_head` u8 → **u16**；
3. **§3.2 vs §10.5 矛盾**：`MAX_XFORM_SLOTS` 统一为 **16**；canned op 清单已定稿 17 个（D4），§10.5 关闭；
4. **§3.3**：`on_died` 回调构想否决（P5），替换为 `death_script` + 相位 9 挂钩（A7）；敌人字段以 D5 为准（双半径、move_to 插值器）；
5. **§3.5**：step 顺序以 A4 v2 为准（spawn_q flush 删除、新增相位 9 挂钩、"boss 阶段推进检查"移出 cleanup）；
6. **§3.7**："世界向外暴露字段"的疑问由 `WorldView` + `globals` + `boss_ui` + `frame_events` 正式解答（A9）；
7. **§4.4**：`create_bullet` 的 `task_script` 参数移到 syscall 绑定层组合（D12 注）；新增 syscall：`pulse_signal / last_status / nearest_enemy / drop_item`；变换序列 locals ABI 细化为每槽 3 字打包（D4）；
8. **§5.1**：`ActionInput.buttons` 建议 u8 → **u16**（A8 跨层备注）;
9. **static_ecl** 概念拆分为 **WorldTables + EclImage**（A3），合并内容哈希语义不变；
10. **§10 待拍板清单**：#5（变换槽/op 清单）关闭；#4（容量）初值已钉（D10）、留实测调参；#6/#3 维持已决。

# Part VI 遗留开放问题

1. `CART_FX` 极坐标回填的**速度阈值具体取值**——实现期定，规则文本进契约（D3）；
2. 容量常数全表的金向量实测验证（D10）；
3. 道具高度计价的 ZUN 机制考证（D7 / Part IV-4）；
4. Q32.32 位置精度（母文档 §10.2，M0 实测说话）；
5. 共场/分场（母文档 §10.1）——对 stg-world 无结构影响（分场 = 每人一 world 实例），维持共场超集设计；
6. shooter 状态载体 locals[64] 容量验证（母文档 §10.7，ECL 层事项，与 D4 的"16 槽 = 48 字"占用共验）。

---

*决策溯源：本文档由 20 轮 grill 评审产出。Q1 边界（P1）→ Q2 职责（A1/A2）→ Q3 step 所有权（P2）→ Q4 单线程（P3）→ Q5 错误策略（P4）→ Q6 池宏（D2）→ Q7 砍 spawn_q（A6）→ Q8 死亡机制（P5/A7/A3）→ Q9 StageState 拆解（A2）→ Q10 自机分工（A8）→ Q11 通道 API（A9）→ Q12 事件系统（A5）→ Q13 运动模型（D3，经 TH16 bullet_tick 逆向修正为双表示）→ Q14 变换系统（D4，经 TH16 弹 VM 逆向升级：wait 门/信号/插值/多槽 arity）→ Q15 敌人池（D5）→ Q16 自机（D6）→ Q17 小池与坐标系（D7）→ Q18 快照/校验和（D11）→ Q19 数学核 newtype（D1）→ Q20 容量与静态表（D10/A3）。*
