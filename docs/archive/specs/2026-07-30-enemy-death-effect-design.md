# 敌人死亡效果可显式调用 —— 设计（2026-07-30）

> 状态：已拍板，待写实施计划。
> 权威度：低于 `design_doc.md` / `stg-world-design.md`；与两者冲突处以两者为准，本文只记本刀决策。

## 1. 问题

敌人的"死亡效果"（掉落 / 事件 / 死亡特效）目前是 `WorldBody::damage_enemy`
（`crates/stg-core/src/world/settle.rs:41-79`）里的一段**内联代码**，只在 `hp <= 0` 时跑。
后果：

- **脚本无法主动触发死亡效果。** 写不出"神风敌到点就地阵亡（带掉落与特效）"，也写不出
  "boss 阶段转换时吐一波道具"。
- **D9 自然退场是完全静默的**（`ecl::vm::run_tasks` 的 `Exec::End` 分支只置 `ENEMY_DYING`）。
  想让"任务跑完的敌人也掉落"，脚本无计可施。
- **敌人的 `score` 字段不给自机加分。** 它只被塞进 `EVT_ENEMY_DIED.data[0]` 与
  `REQ_ENEMY_DEATH` 供表现层显示。打死敌人的**全部**收益来自掉落物被拾取
  （`credit_item`）。`spawn_enemy` 的 `score` 参数目前是纯装饰。

## 2. ZUN 的同类机制（参照，非照抄）

TH13 指令表（数据源：Priw8 的 ECL 文档 `js/ecl/ins.js`；TH17 助记名取自 ECLjs 仓库的
`th17.eclm`。本仓 `docs/zun-ecl-v2-reference.md` 只录了 VM 级 ins_0-93，游戏指令段未录，
故本节是新查的）：

| ins | 助记 | 参数 | 原文语义 |
|---|---|---|---|
| 506 | `dropClear` | — | Clears caller's extra item drop. |
| 507 | `dropExtra` | type, amount | Adds %2 items of type %1 to caller's extra drop. |
| 508 | `dropArea` | w, h | Sets caller's item drop area width to %1 and height to %2. |
| 509 | `dropItems` | — | **Drops all items the caller has.** Has no effect in spell practice. |
| 510 | `dropMain` | type | Sets caller's main drop to %1. |
| 556 | `setDeath` | sub | 设死亡 sub。 |
| 561 | `die` | — | **Makes the caller execute the sub set with 556. If there is no such sub set, the caller dies instead (exactly the same as if the player shot down the caller, that is, items are dropped, death sound plays etc).** |

要点三条：

1. **掉落是敌身上可增量配置的可变状态**，不是生成时定死的一张表。
2. **`dropItems` 与死亡解耦**——"把身上带的撒出去"是个独立动作。
3. **`die` 经 `setDeath` 间接一层**：设了死亡 sub 就跑它，没设才走默认全套死亡。

## 3. 决策（人类裁定，实施不得偏离）

| # | 决策 | 备注 |
|---|---|---|
| D-1 | **照 ZUN 拆两件**：`drop_items()`（撒掉落）+ `die()`（走完整死亡效果） | 对应用户的两个用例 |
| D-2 | **做增量掉落配置**：`drop_clear()` / `drop_add(type, n)` | 敌身上多一份可变掉落状态 |
| D-3 | **`drop_items()` 吐完不清空计数**（照 ZUN 字面） | 代价：`drop_items(); die();` = 双份掉落，作者自负 |
| D-4 | **`die()` 立即终止调用它的任务** | 后续语句不执行 |
| D-5 | **死亡效果强制加分**：把 `enemies.score[e]` 记进自机分数 | 新行为，金向量会漂 |
| D-6 | **`death_script` 不在本刀通电** | ZUN 的 556/561 间接层留给将来独立一刀 |
| D-7 | 砍 `dropMain` | 我们无"主掉落"概念，`drop_clear` + `drop_add` 已覆盖 |
| D-8 | 砍 `dropArea` | 开放散布要动 RNG 消耗形状，独立一刀 |
| D-9 | **dying 敌当帧仍参与碰撞**，不特殊处理 | 见 §6 |

## 4. 架构

### 4.1 池字段：一个真相源

```
EnemyPool:   drop_table: u16   →   drop_count: [u8; ITEM_TYPE_COUNT]     // ITEM_TYPE_COUNT = 5
```

`drop_table` **不再是存储状态**，退化成 `spawn_enemy` 的一个**参数**：生成时展开进
`drop_count`，此后无人读它。

- **脚本面不变**——`.ecl` 作者照旧 `spawn_enemy(..., drop_table, ...)` 传表号。
- Rust 侧 `EnemyInit` 换字段（`define_pool!` 的 Init 是 exhaustive，漏字段编译不过），
  约 10 处调用点机械改；配一个助手（见 §4.4）。
- **为什么不保留两份**（ZUN 的 main drop + extra drop）：我们的 `drop_table` 是个**多重集**
  而非单个主道具，硬套会得到两套语义打架的掉落来源。ZUN 的 `dropMain` 因此一并砍掉（D-7）。

容量账（D10）：`-2 B`（u16）`+5 B`（`[u8;5]`）= **+3 B/敌 × 256 = +768 B**。
`Checksum` 与 `SaveBytes` 都有 `impl<T, const N: usize> for [T; N]`
（`checksum.rs:139` / `save.rs:100`），故 `[[u8;5]; 256]` 自动进校验和与存档，无需新代码。

### 4.2 提两个函数

```rust
/// 只撒掉落：按**类型升序**（I4）逐个 spawn_drop。不清零、不加分、不发事件。
pub(crate) fn spill_drops(&mut self, e: usize, tables: &WorldTables);

/// 完整死亡效果。**幂等**：已 ENEMY_DYING 即直接返回。
///   ① hp = 0（状态自洽）
///   ② flags |= ENEMY_DYING
///   ③ spill_drops
///   ④ players[0].score = saturating_add(enemies.score[e])      ← 新（D-5）
///   ⑤ push EVT_ENEMY_DIED
///   ⑥ emit REQ_ENEMY_DEATH
pub(crate) fn kill_enemy(&mut self, e: usize, tables: &WorldTables);
```

调用方：

| 函数 | 调用方 |
|---|---|
| `kill_enemy` | `damage_enemy` 的 `hp<=0` 分支（原内联代码搬过去）／ `SYS_DIE` |
| `spill_drops` | `kill_enemy` ／ `SYS_DROP_ITEMS` |

**加分对象是自机 0**——与既有 `SYS_ADD_SCORE` 同口径（`ecl-ops.md` 号表 50 行明写"自机 0 记分"）。
`enemies.score` 是 `u16`，`players[].score` 是 `u64`，用 `saturating_add`。

**`kill_enemy` 置 `hp = 0`**：`die()` 可能打在满血 boss 上，不置零会让 HUD 当帧显示"满血的死人"。
注意这**绕过**了 `damage_enemy` 里的符卡血线下钳——这是有意的：`die()` 是脚本的显式动作，
应当压过为伤害路径设的保护。

### 4.3 四个 syscall（号 58/59/60/61）

54 = `clear_bullets`、55/56/57 = `add_lives`/`add_bombs`/`add_power` 已占（ECL 复刻刀），
本刀 append-only 占 58-61。

| 号 | 表层内建 | 参数 | 语义 |
|---|---|---|---|
| 58 | `drop_clear()` | — | `drop_count` 全归零 |
| 59 | `drop_add(type, n)` | type, n | `drop_count[type]` 加 `n`（先把 `n` 钳进 `[0, u8::MAX]`，再 `saturating_add`——见 §7） |
| 60 | `drop_items()` | — | `spill_drops`（**不清零**，D-3；**不设 dying 门禁**——对已 dying 的敌照撒不误） |
| 61 | `die()` | — | `kill_enemy` + 终止本任务（D-4，见 §4.5） |

四个都**无返回值**；**self owner 必须是 ENEMY**。

### 4.4 掉落表展开助手

```rust
// crates/stg-core/src/tables.rs
/// 把掉落表号展开成逐类型计数。越界表号 → 返回全零 + `false`（调用方据此计 contract_viol）。
pub fn drop_counts(tables: &WorldTables, table: u16) -> ([u8; ITEM_TYPE_COUNT], bool);
```

两个消费者：`sys_spawn_enemy`（把第 4 参展开）、harness/测试的 Rust 侧建敌。

原先在 `settle.rs:44-46` 的越界 `drop_table` P4-b 检查（follow-ups B11 的测试
`settle_out_of_range_drop_table_degrades_to_empty`，`settle.rs:421`）**移到展开处**，
等价保留；那条测试改成测生成路径。

### 4.5 `die()` 的表层降低——不动 VM

`syscall::dispatch` 的签名是 `Result<(), u8>`，没有"结束本任务"的返回通道。与其给六十个
match arm 换返回类型，不如让编译器把表层 `die()` 降低成**两条指令**：

```
SYS(SYS_DIE)      // 跑死亡效果
OP_KILL_SELF      // 已有 op，exec 里 `return Exec::End`（vm.rs:372）
```

零 VM 改动、零 op 表改动，与既有的 `mark` 垫片降低同套路。

**与 D9 的交互**：若 `die()` 发生在敌的**主任务**里，`OP_KILL_SELF` 的 `Exec::End` 会让 D9
再标一次 `ENEMY_DYING` 并清 `main_task`——`kill_enemy` 幂等，无害。

**已知窄面**：raw builder 使用者可以只发 `SYS_DIE` 而不发 `OP_KILL_SELF`，得到"敌已死而任务
仍在跑"。表层作者碰不到（`die()` 是一体降低的），文档记一句即可。

### 4.6 道具类型常量入 `consts.rs`

`drop_add(ITEM_POWER, 5)` 需要类型名对脚本可见。放**①结构常量**段
（`ENGINE_STRUCTURAL`）而非②表符号段：`items.rs:9` 明写类型编号是**冻结**的，且
`credit_item` 里每种类型的语义是硬编码的——不像弹型/色名那样可被 mod 扩展。

登记五个：`ITEM_POWER=0` / `ITEM_POINT=1` / `ITEM_LIFE_PIECE=2` / `ITEM_BOMB_PIECE=3` /
`ITEM_STAR=4`。

## 5. 数据流与时序

四个 syscall 都在**相位 2**（ECL 导演槽）跑。

**掉落的生成时机有两种，都确定但不同**：

| 路径 | 生成相位 | 当帧可见性 |
|---|---|---|
| `die()` / `drop_items()` | 2 | 当帧被相位 5 积分、相位 6 参与碰撞 |
| 伤害致死 | 7 | 下一帧才动 |

**敌人回收**：两条路径都只置 `ENEMY_DYING`，相位 9 `cleanup`（`cleanup.rs:53`）统一回收。

**符卡**：`die()` 打在绑卡 boss 上，经现有的破卡三路 OR（`spell.rs:106-110`，其中一路正是
`ENEMY_DYING`）自动收卡结算——**无需新增机制**。

**子任务**：`die()` 只终止调用它的那个任务。同 owner 的其他任务在下一帧被 `run_tasks` 的
owner-gate 发现 owner 已死后静默清杀（现有机制）。

## 6. 已知行为：dying 敌当帧仍参与碰撞

`collide`（相位 6）**不看** `ENEMY_DYING`，只有 `settle` 趟二的伤害臂看。所以一只 `die()`
掉的敌人当帧**体碰仍然成立，仍能撞死自机**。

这与伤害致死同性质（那边 dying 在相位 7 才置，相位 6 早已碰完），只是 `die()` 把窗口拉长到
整帧。**决定不特殊处理**（D-9）：加一条"dying 不参与碰撞"的例外会让碰撞矩阵多一条隐式规则，
而"神风敌撞死你"本就说得通。**此条必须进 `ecl-lang.md`**。

## 7. 错误处理（P4 三铁律）

| 情形 | 处置 | 依据 |
|---|---|---|
| 非 enemy-owner 调这四个 | **Fault**（`FAULT_BAD_OP`） | 照 `self_enemy_handle`（`syscall.rs:222`）既有 misuse 口径，不自创 |
| `drop_add` 类型 ≥ `ITEM_TYPE_COUNT` 或为负 | no-op + `contract_viol` +1 | P4-b |
| `drop_add` 的 `n` 为负或超 u8 | 先 `n.clamp(0, u8::MAX as i32)`，再对计数 `saturating_add` | P4-b；`n` 是脚本给的任意 `i32`。**两步都要**：只 clamp 不饱和会在计数接近 255 时溢出，只饱和不 clamp 则 `as u8` 会把负数回绕成大正数。裸 `+` 在 debug 下 panic（B20 那刀的教训）。本刀不做"减掉落" |
| 道具池满 | `spawn_drop` 自身逐颗降级计 `pool_full[POOL_ITEM]` | P4-a，现成 |
| `die()` 对已 dying 的敌 | 幂等 no-op | 与 settle 趟二的 dying 门禁同构 |
| 生成时 `drop_table` 越界 | 视同空表 + `contract_viol` +1 | §4.4，等价搬迁 |

## 8. 测试策略

**金向量会漂，而按 CLAUDE.md「金向量闸门的能力边界」它本就抓不到行为回归**——本刀的正确性
完全靠单测。以下每条都要有判别力，不能是重言：

| 测试 | 判别腿（防什么错误实现） |
|---|---|
| 打死敌人 → `players[0].score` 恰好 `+= enemies.score[e]` | **测试内不拾取任何道具**——否则分不清是敌人加的还是道具加的。D9 那刀正是在这里栽过（"score 未变"断言因分数走 `credit_item` 而失效） |
| D9 自然退场仍**不**加分**不**掉落 | 反向腿：防实现者把 `kill_enemy` 顺手挂到 D9 的 `Exec::End` 上 |
| `drop_items(); die();` → **双份**掉落 | 钉死 D-3。防将来有人"顺手修好"成清零语义 |
| `die();` 后续语句不执行 | 钉死 D-4 |
| `die()` 两次 → 掉落与加分各只一次 | 幂等 |
| 掉落按类型升序生成 | I4；用一个跨多类型的 `drop_count` 造判别力 |
| `drop_add` 越界类型 → `contract_viol` +1 且无掉落 | P4-b |
| 非 enemy-owner 调四者 → Fault | misuse 口径 |
| 道具池满 → 逐颗降级、不 panic、计数 | P4-a |
| `die()` 打满血 boss → `hp` 归 0 | 钉死 §4.2 ①，防"HUD 当帧显示满血死人" |
| `die()` 打绑卡 boss → 收卡结算 | §5 的"无需新增机制"是个断言，要验 |

**掉落顺序不漂的论证**：内建掉落表 1 是 `[(ITEM_POWER,2),(ITEM_POINT,1)]`，而
`ITEM_POWER=0 < ITEM_POINT=1`——表序恰好就是类型升序，故改成按类型计数后
`spawn_drop` 的 **RNG 消耗顺序不变**。金向量的漂移**只**来自死亡加分与池布局，不来自掉落顺序。
（这条要在实施时**实测确认**，不能只靠推理。）

## 9. 破坏面与收口清单

- **`ENGINE_VER` 3 → 4**（syscall 号表变更）；`step.rs` 的锚点测试同步。
- **金向量整体平移**，两个独立原因：① 死亡加分（行为）② 池字段布局变化（校验和）。
  commit 里要写清是这两个，不能含糊成"金向量变了"。
- `EnemyInit` 换字段 → 约 10 处调用点。
- 文档：
  - `docs/ecl-lang.md`：四个新内建的语义 + **死亡语义一节**（`die()` vs 自然退场 vs 被打死的
    三者差异）+ §6 的体碰坑 + D-3 的双份掉落坑。
  - `docs/ecl-ops.md`：syscall 号表追 58-61 四行。
  - `stg-world-design.md`：敌人字段表 `drop_table` → `drop_count`（权威文档，改动过评审）。
  - `docs/follow-ups.md`：本刀若产生新债在此记。
- 两个冒烟脚本 + `verify-tables` + 两个 `.ecl` check 全绿。

## 10. 明确不做

- `death_script` 通电 / ZUN 的 `setDeath` 间接层（D-6）——独立一刀。本刀落地后与它叠加，
  即与 ZUN 的 561 完全同构。
- `dropMain`（D-7）、`dropArea`（D-8）。
- "减掉落"（负 `n`）。
- 读回 `drop_count` 的查询 syscall（YAGNI）。
- 让 dying 敌退出碰撞（D-9）。

## 11. 顺带发现（不在本刀，记档）

`spell.rs:472-480` 的注释论证"`hp_break` 三路 OR 里的 `ENEMY_DYING` 那路无法被判别式测试
覆盖"——因为当时唯一置 `ENEMY_DYING` 的路径是 `hp<=0`，与第三路 `hp<=threshold` 恒同真。
**D9 落地后这个论证已经过时**（自燃路径会在 hp 远高于 threshold 时置 dying），本刀的 `die()`
更是如此。该注释应择机订正，并补一条真正判别 `ENEMY_DYING` 那路的测试。
