# 道具池（掉落 / 吸引 / 拾取入账）—— 设计 spec

> 状态：已过 brainstorm 拍板，待实施计划。上游设计：`stg-world-design.md` D7（池字段/行为链/
> 计价 v1）、D8 行 5、D9 趟三、A6/A7（掉落直接分配、散布消耗世界 RNG）——均为既有评审拍板，
> 本 spec 只补实现决策。milestone 候选名 **M0-12**。
> **本 spec 获批 = 碰撞矩阵行 5 启用的评审**（CLAUDE.md 自检第 3 条）。

## 拍板纪要（2026-07-16）

1. **类型集合**：`POWER` / `POINT` / `LIFE_PIECE` / `BOMB_PIECE` 四类 + **满 power 转化**
   （power 已达上限时 POWER 道具按 POINT 分值入 score——ZUN 经典规则）。
2. **配置表 v0 = 引擎常量表**（`items.rs`），沿 `player.rs`"暂 const、WorldTables 将来接管"先例。
3. **吸引三源一机制**：近距磁吸圈 / PoC 越线 / 通用入口 `attract_all_items`——殊途同归写
   `magnet_to`；目标非 ALIVE 即解锁回落。
4. **扩展性是一等设计目标**（用户点名）：新增道具类型的成本 = 一行表 + 一条入账臂，见下节。

## 扩展性设计（新增类型的标准动作，照抄 D2"新增池五步"的体例）

**新增一种道具类型 = 四步，无隐藏步骤：**

1. `items.rs` 加类型常量 `pub const ITEM_XXX: u8 = N;`（编号只增不改，有钉死测试押运）；
2. `ITEM_CFG` 常量表加一行（`ItemTypeCfg` 全字段——**表长 == 类型数有编译期断言**，
   加常量忘加表行编不过，复用"exhaustive Init"哲学）；
3. `settle` 趟三的**唯一入账函数** `credit_item(player_idx, item_type)` 加一条 match 臂
   （全部 per-type 逻辑的唯一居所——跨类型规则如满 power 转化也住这里；未知类型走
   `_ =>` P4-b 计数忽略，两机同弃）；
4. 需要进掉落的话给掉落表加行/加表。

**为将来铺的口子（本刀不实现，只保形状）**：
- `ItemTypeCfg` 结构体按"将来整表烘进 WorldTables"设计——搬家时结构体不动、存储搬家；
- `timer: u16` 字段本刀惰性（随快照入校验和）——将来闪烁/限时消失类道具（如收集失败会
  消失的 point）直接启用，池布局不变；
- 特殊行为类道具（如"全场转点"）= 入账臂里写世界侧代码（**数据选行为**，P5 禁回调的正解）；
- 磁吸圈/磁吸速度 per-type 而非全局——将来"磁铁道具"这类差异化吸引直接改表行。

## 池与常量

- **`ItemPool`**：`define_pool!` 第五实例，cap **512**（D10 已预算 11KB），字段照 D7 定稿：
  `x, y, vx, vy: Fx` / `item_type: u8` / `magnet_to: u8`（0xFF = 未锁定）/ `timer: u16`。
  `POOL_ITEM: usize = 5`（4 被 POOL_XFORM 占用）。
- **类型编号（冻结，钉死测试）**：`ITEM_POWER = 0`、`ITEM_POINT = 1`、`ITEM_LIFE_PIECE = 2`、
  `ITEM_BOMB_PIECE = 3`；`ITEM_TYPE_COUNT = 4`。
- **`ItemTypeCfg`**（v0 常量表行）：`score: u32`（v1 固定分值）、`eject_speed: Fx`（弹出初速）、
  `terminal_vy: Fx`（终端速度）、`magnet_speed: Fx`（磁吸速度）、`pickup_radius: Fx`（D8 行 5
  主动半径）、`attract_radius: Fx`（近距磁吸圈）。
- **默认参数**（全部"金向量实测后调参"门）：弹出初速 3.0 向上、重力 `ITEM_GRAVITY = 0.15/帧²`
  （全局常量）、终速 2.2、磁吸 8.0、拾取半径 16、磁吸圈 40；分值 POWER=10、POINT=100、
  蜡类=50。`POC_LINE_Y = 128`（游戏配置段常量，世界侧）。
- **入账常数**：`POWER_MAX = 128`、`PIECES_PER_LIFE = 5`、`PIECES_PER_BOMB = 5`。

## 运动与吸引（integrate，敌人之后、fields 之前——文档相位序）

无显式状态位，物理即状态：

```
未锁定（magnet_to == 0xFF）：
    vy < terminal_vy → vy += ITEM_GRAVITY（越过即钉在 terminal_vy）；x 不受力（vx 保持弹出散布值）
    pos += vel
锁定（magnet_to == p）：
    players[p] 非 ALIVE → 解锁（magnet_to = 0xFF，回落，vx 清零 vy 保持——从当前位置自然续落）
    否则：angle = atan2(dp)，vel = polar_to_vec(magnet_speed, angle)，pos += vel（每帧重瞄，直线急追）
```

**触发写 `magnet_to`（都在 integrate 道具趟内，判定先于本帧移动）**：
1. **近距**：`len_sq(自机−道具) <= attract_radius²`（i64 平方距离，最近 ALIVE 自机、并列低索引 I4）；
2. **PoC**：存在 ALIVE 自机 `y < POC_LINE_Y` → 本帧全场未锁定道具锁定它（多自机过线取低索引）；
3. **`attract_all_items(player: usize)`**：公开写 API——坏索引/非 ALIVE 目标 → P4-b no-op + 计数；
   遍历全场未锁定道具上锁。bomb / 导演 / 将来 ECL syscall 的通用入口。

掉落当帧只过 cleanup、**次帧首动**（既定语义，与弹一致）。

## 掉落

- **掉落表 v0**（`items.rs` 常量）：`DROP_TABLES: &[&[(u8, u8)]]`——表 id → [(类型, 数量)]。
  表 0 = 空（不掉，敌人 `drop_table` 零初始化默认）；表 1 = 标准杂鱼（2×POWER + 1×POINT）。
  越界表 id → P4-b 计数 + 视同空表。
- **散布（世界 RNG，消耗序 = 结算序——A6 既定）**：每颗 `vx = rand ∈ [−1, +1]`、
  `vy = −eject_speed + rand 抖动 ∈ [0, 0.5]`（具体取 rand_range 整数域实现，计划钉死）。
- **接线**：settle 趟二死亡结算（现有 `EnemyDied` 产出点）按 `enemy.drop_table` 展开掉落——
  调用内部 `spawn_drop(x, y, item_type)`；公开 API **`drop_item(x, y, item_type) -> ItemHandle`**
  走同一条散布代码路径（池满 → NULL + `pool_full[POOL_ITEM]`，P4-a；坏类型 → NULL + BAD_ARGS）。

## 拾取与入账

- **D8 行 5 接线**（本 spec 获批即评审）：`ROW_ITEM_PLAYER = 5`，主动 = item（`pickup_radius`
  查配置表），被动 = `player.graze_radius`（兼拾取圈，D7 既定）。collide 只收集；**自机非可拾
  状态（非 ALIVE）不收集**（与行 1/2 的自机门禁同款）。
- **趟三入账**：`credit_item(p, item_type)`——唯一 per-type 逻辑居所：
  - `POWER`：`power < POWER_MAX` → `power += 1`；**已满 → `score += cfg(POINT).score`**（转化）；
  - `POINT`：`score += cfg.score`；
  - `LIFE_PIECE`：`life_pieces += 1`；`== PIECES_PER_LIFE` → 清零、`lives += 1`；
  - `BOMB_PIECE`：同构进 `bombs`；
  - 未知类型：P4-b 计数忽略。
  道具打 `ITEM_PICKED` 标记（flags？——道具池无 flags 字段：**用 `magnet_to = 0xFE` 哨兵标记
  已拾取**，cleanup 回收；一颗道具同帧只能被拾取一次（行 5 每对至多一 hit + 趟三首见即标））。
  产出事件 `EVT_ITEM_PICKED = 4`（`a_index/a_gen` = 道具句柄位，`data[0] = item_type`，
  `data[1] = 玩家号`）。
- **cleanup**：已拾取（0xFE）或越界（沿用 OOB 判定；主要是掉出底部）→ 回收。

## 金向量与测试

- **金向量加戏**：导演敌人 `drop_table = 1`（现有死亡即自动掉落，散布 RNG 入对拍）+
  每 200 帧 `attract_all_items(0)` 全场磁吸 + 自机既有走位自然拾取。双跑照旧。
- **判别式测试重点**：满 power 转化分支（power 恰 128 时 POWER 入 score 且 power 不动）/
  蜡进位跨界（4+2 → lives+1 且 pieces==1）/ PoC 触发与解锁（目标死亡回落）/ 磁吸每帧重瞄
  几何（非圆心重合）/ 行 5 判别几何 / 掉落 RNG 消耗序（两敌同帧死，掉落顺序 = 结算序）/
  次帧首动 / 类型编号钉死 + 表长编译期断言 / **池满 P4-a**（顺带偿还 follow-ups B1 的道具池
  份额——`drop_item` 打满 512 断言 NULL + 计数）。
- 合入前变异检验 ≥3（进位 off-by-one / 转化分支阉割 / 磁吸圈比较反向 三个候选）。

## 收尾义务

设计回写 `stg-world-design.md` D7（配置表 v0 落点、磁吸圈列、0xFE 哨兵、四步扩展清单指针）；
`docs/follow-ups.md` B1 更新（道具池份额已还，条目改写剩余三池）；`PROGRESS.md` 史行 + 现在段；
CLAUDE.md 结构树加 `items.rs`；`docs/xform-ops.md` 不涉。新增 World 字段仅 `items` 池
（derive 自动入校验和 + copy_into 加行 + 快照往返测试）。

## 验收

判别式单测全绿且过变异检验；金向量三平台互比全等；clippy/fmt 零告警；`cargo tree` 防火墙不变。
