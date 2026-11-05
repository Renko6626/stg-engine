# 消弹区（Field）设计 —— 通用圆形作用区原语

**日期**：2026-07-15
**状态**：已评审通过（用户拍板），待写实施计划
**前置**：M0-7（碰撞 D8 四行 + 结算 D9 三趟 + 生死状态机）已合入 main

## 目标

给世界层一个**通用的圆形作用区原语**：创建一个圆（本帧存在或持续 N 帧），用标准圆碰撞判定，
执行**消弹**（和可选的**伤敌**）。bomb 只是它的首个租户，不是它的主人。

## 定位与原则

`FieldPool` = **哑数据原语**，世界层零内建行为：

- **静止**。世界层不做 follow。**跟随** = 上层每帧在目标位重铺 `life=1`；**静止爆炸** = 铺一次 `life=N`。
  同一原语两种用法、零分支。
- **通用**。任何持有 `&mut WorldBody` 的租户都经 `create_field` 写 API 创建（P1）：
  角色模块（bomb）、ECL 主控（符卡切换/阶段清场）、ECL 死亡脚本（敌人死亡清弹——设计既定
  「敌人死亡**默认不清弹**」，即必须脚本显式要）、导演槽（金向量/测试）。
- 这条路线延续项目已拍板两次的同一原则：**取消静态 proto → 哑创建原语 + 脚本层 shooter**；
  **创建原语不内建 RNG → 随机散布交 ECL 循环**。世界层出哑数据，行为交上层组合。

## 数据

### FieldPool（`define_pool!` 第 4 个实例）

```rust
define_pool! {
    Field, cap = 16,
    fields {
        x: Fx, y: Fx, radius: Fx,
        dmg_per_frame: u16, life: u16,
        owner: u8, flags: u8
    }
}
```

`owner`：字段留存（D6 原设计已有），**本刀无消费者**——co-op 记分与道具计价都未实现，
语义待那一刀定。

### 常量

```rust
// 能力位（flags）：collide 按位决定启用哪一行，未启用的行在收集前就跳过
pub const FIELD_CLEAR_BULLETS: u8 = 1 << 0;  // 行 6 消弹
pub const FIELD_DAMAGE:        u8 = 1 << 1;  // 行 7 伤敌

// 半径
pub const FIELD_RADIUS_FULLSCREEN: Fx = Fx::from_int(400);   // 覆盖全场含边距
pub const FIELD_MAX_RADIUS:        Fx = Fx::from_int(1024);  // create_field 钳制上限

// 弹 flags 位定义开张（此前 bullets.flags 一个位都未定义）
pub const BULLET_CLEARED: u8 = 1 << 0;
```

**`FIELD_RADIUS_FULLSCREEN = 400` 的推导**：场界 x∈[-192,192]、y∈[0,448]，越界边距 64
→ 弹最远可在 (±256, −64..512)。field 置场心 (0,224) 到最远角的距离
= √(256² + 288²) = √148480 ≈ **385.3 px** < 400。给脚本一个算好的常量，免得各自去猜"多大算全屏"。

**`FIELD_MAX_RADIUS = 1024` 的动机（定点数安全）**：`Fx` 上限 32767.99998。若脚本传 32767
表达"无限大"，`field.radius + bullet.radius` 的 **Fx 加法溢出** → debug panic / **release 回绕成负数**
→ `sum.raw() as i64` 为负、平方仍是正的巨数 → **全场无条件判撞**。这是 debug/release 分歧类。
钳到 1024 后 `1024 + 16 ≪ 32767`，Fx 加法永不溢出，行 6/7 得以与行 1-4 保持完全一致的写法
（`(a + b).raw() as i64`），不必为 field 特设 i64 加法。

## 相位数据流

```
相位2/3  导演·ECL·角色模块 → create_field(FieldInit)     ← 钳制 radius（P4-b）
相位5    integrate  → field.life -= 1（照抄弹的模式）
相位6    collide    → 行6 Field×EnemyBullet（gate: flags & CLEAR_BULLETS）
                       field 外层↑ × 弹 内层↑；delay>0 的弹跳过
                     行7 Field×EnemyBody（gate: flags & DAMAGE）
                       field 外层↑ × 敌 内层↑；**不查敌 invuln**（事件照收、结算时判）
                     —— 纯读、只 append hits（硬规则）
相位7    settle 趟一 → 行6：未清除的弹置 BULLET_CLEARED，per-field 计数
                       扫完按 field 索引↑ 发 FieldCleared{field, count, x, y}
              趟二 → 行7：敌人扣 dmg_per_frame（已 dying 跳过=overkill；invuln≠0 跳过）
                       → hp≤0 → 置 dying + EnemyDied（只发一次）——与行4 同款门禁
                     行1 中弹【新增：跳过已清除的弹】← bomb 救命的实现
                     行3 中弹（敌体，无"清除"概念）
              趟三 → graze 照算，不查已清除位（见下）
相位9    cleanup    → 已清除弹回收；field life==0 回收
```

**`life=1` 恰好活一帧且当帧生效**：相位5 减到 0 → 相位6 alive 位仍在、照常参与判定 → 相位9 回收。
这是"每帧重铺 = 跟随"能成立的时序基础。

**碰撞判定**（与行 1-4 同款，D8/I1）：`len_sq(dx,dy) <= (r_active + r_passive)²`，
i64 Q32.32 同域直接比、不开根、不归一化回 `Fx`。
行6 半径映射 = `field.radius` + `bullet.radius`；行7 = `field.radius` + `enemy.hurtbox`。

## 关键语义决策

### 消弹×graze：算（趟三不加门禁）

被消弹区清掉的弹，**同帧仍计 graze**。

- **碰撞检测在相位 6 发生**，那一刻弹是活的、确实进了擦圈；清弹是相位 7 才做的事，**晚于检测**。
  "擦在先、清在后"。
- 趟二跳过已清除弹是**保护**语义（bomb 救你不死）；趟三是**计分**语义。设计明写 graze
  「独立于中弹」，两者不连坐。
- **手感**：bomb 按下前那一瞬的擦弹本就该算——玩家确实冒了那个险。若不算，等于"放 bomb
  倒扣擦弹分"。
- 实现上零代码（趟三保持现状）；`grazed_by` 逐弹一次天然兼容（弹被清了也不会再擦第二次）。

### 趟一必须"标记"不能 free

与敌人 dying 同款理由：趟二/趟三随后要按索引读这颗弹（跳过判定、graze 记账），
趟一若当场 `free_index`，后两趟读的就是回收槽。回收统一由 cleanup（相位9）做。

### 幂等：多 field 同帧压同一弹

趟一按 hits 收集序扫，第一个 field 打标记 + 计数，后续 field 见 `BULLET_CLEARED` 位就跳过。
弹只消一次、只被计一次。

## 三处对权威设计的偏离（已评审通过）

### 1. `BombFieldPool` → `FieldPool`（正名 + 能力位）

原设计（stg-world-design.md D6）命名为 `BombFieldPool`、注明"角色模块创建"——这是**按首个用户命名**。
但实际租户远不止 bomb：boss 符卡切换清弹（design_doc.md §"血量过阈值或超时 → 清弹、结算、宣言下一张
符卡"）、ECL 阶段清场（design_doc.md §"清弹（bomb/阶段清场）是独立机制"）、敌人死亡脚本清弹。
这些都不是"角色模块"，不该借道 bomb 状态机去铺。

保留**双能力**（消弹 + 伤敌，即矩阵行 6/7 同一主动方）——东方 bomb 本就同时消弹与打 boss，
拆成两个池会逼 bomb 每次铺两个同心圆并同步两份寿命。纯消弹区（符卡切换）只开 `CLEAR_BULLETS` 位。

用 `flags` 位而非 `dmg_per_frame == 0` 当伤敌开关：意图显式，且 collide 能在**收集前**跳过整行
（省 O(N×M)，而非收集完到趟二扣 0 血还白发一个事件）。

### 2. 逐弹 `BulletCleared` → 聚合 `FieldCleared`

原设计（D9 趟一）："被清弹打'已清除'标记 + `BulletCleared`"。**这在全屏消弹下必爆容量**：

| 缓冲 | cap | 全屏消弹最坏（8192 弹全在场） |
|---|---|---|
| `frame_events` | 512 | 逐弹发 → 溢出 16×，丢 7680 条 |
| `reqs` | 256 | 逐弹发 → 溢出 32×，丢 7936 条 |

且 A5 的 events 生产者清单里**根本没列 BulletCleared**（只有 EnemyDied/PlayerDied/PlayerBombed/
ItemPicked）——说明原设计未核过这笔账。

改为**每个 field 每帧一条**聚合事件：

```rust
pub const EVT_FIELD_CLEARED: u8 = 3;
// Event { kind: EVT_FIELD_CLEARED, a_index: field_idx, a_gen: field.generation,
//         x: field.x, y: field.y, data: [count, 0] }
```

field cap 16 → **≤16 条/帧，永不爆**。`count`（本帧本 field 消了几颗）是廉价且有用的事实：
将来消弹转分、统计、表现层决定特效强度都要它。

表现层不丢信息：弹从池里消失了自然就不画（MultiMesh 每帧从池读）；**逐弹消失特效**将来由**道具**
承担（真东方消弹即每颗弹变星星道具——那是道具池的实体，不是特效请求）；field 的圈特效等 `reqs` 那刀。

### 3. collide 的「已清除弹不参与检测」不实现

原设计（A4 相位表 / D8）："`delay > 0` 与已清除标记的弹**不参与检测**"。

**这条在当前相位序下不可达**：`BULLET_CLEARED` 由趟一（相位7）置，由 cleanup（相位9）回收，
**同帧完成**——次帧的 collide（相位6）根本看不到任何已清除的弹。该位的整个生命只在
**同一帧的相位 7→9 之间**（趟一置、趟二读、cleanup 读），从不跨帧。

故不实现；写了就是死代码。`delay > 0` 那半条照常实现（已在 M0-7 落地）。

**若将来消弹改为延迟回收**（例如为播消弹特效留几帧），这条就变为可达，届时需补。
本 spec 记录此前提，供后人判断。

## 确定性与错误处理

- **I4 顺序**：field 池遍历升序；行6/7 嵌套固定为 **field 外层↑ × 被动内层↑**。
- **聚合事件的确定序**：用栈上 `counts: [i32; FieldPool::CAP]`（16×4B = 64B）累计，扫完
  **按 field 索引升序**发事件。**不**依赖"row-6 hits 按 field 连续"——那会把 settle 的聚合
  与 collide 的循环结构耦死，将来动 collide（如换 broadphase）就会静默产出多条事件。
- **P4-a 资源耗尽**：field 池满 → `FieldHandle::NULL` + `diag.pool_full[POOL_FIELD]` + `last_status`；
  events 满 → 丢弃 + `diag.events_overflow`。均确定性降级、不 panic。
- **P4-b 调用方违约**：`radius > FIELD_MAX_RADIUS` → 钳制到上限 + `diag.contract_viol`。
  两机同样钳制 → 确定。
- **P6 全量校验**：`FieldPool` 由 `define_pool!` 自动 derive Checksum，无需手工接线。
  `bullets.flags` 的新位是既有字段，已在校验和内。

## 对既有代码的改动

- **趟二的中弹 arm 必须拆开**：M0-7 把 `ROW_BULLET_PLAYER_HIT | ROW_BODY_PLAYER_HIT` 合成一个 arm。
  现在行1（敌弹）需要"跳过已清除的弹"，行3（敌体）不需要（敌人没有"被清除"概念，它们经 hp≤0 → dying）。
- **趟一占位注释兑现**：M0-7 留的 `// 趟一 · 清除/防护：bomb 清弹（行 6）—— 本切片无 bomb，空`
  及其后附的"bomb 落地时须先回答『被 bomb 清掉的弹还该不该算 graze？』"——本 spec 已回答（算），
  注释相应更新。
- **`WorldBody` 加 `fields: FieldPool`**；`copy_into` 加 `s.fields.copy_into(&mut d.fields)`；
  `POOL_FIELD: usize = 3`。
- **integrate 加 field 段**（`life -= 1`）；**cleanup 加 field 段**（`life == 0` → free）
  与**弹的已清除回收**（`flags & BULLET_CLEARED != 0` → free）。

## 测试策略

吃 M0-7 的教训——**金向量闸门只抓跨平台分歧、不抓行为回归**（`ci.yml` 只把三平台互比、无 committed
基线），故行为正确性只能靠单测守，且招牌不变量必须有**判别式**测试（圆心重合式测试对半径映射是瞎的，
M0-7 变异检验已实证）。

- **判别式几何，禁止圆心重合**：field 半径 20 + 弹半径 2 = 和 22 → 弹放 **21px 撞 / 23px 不撞**。
- **能力位判别**：只开 `CLEAR_BULLETS` 的 field 压着敌人 → 敌人 hp **不掉**；
  只开 `DAMAGE` 的 field 压着弹 → 弹**不消**。
- **bomb 救命（招牌语义）**：弹压在自机身上 + 同帧 field 消它 → 自机**不进 DeathWindow**
  （证明趟一先于趟二）。
- **graze 照算**：同上场景 → `graze` 仍 +1。
- **幂等**：两个 field 压同一颗弹 → 该弹只被计一次，两条事件的 `count` 合计为 1。
- **钳制**：`create_field(radius = 30000)` → 实际存 1024 + `diag.contract_viol` +1。
- **聚合事件序**：多个 field 同帧消弹 → 事件按 field 索引升序。
- **金向量**：导演周期性铺全屏 field（`FIELD_RADIUS_FULLSCREEN`），压消弹标记/回收 churn 与
  聚合事件路径。

## 范围边界（本刀不做）

- **bomb**：`BTN_BOMB` 输入位、`bomb_phase/bomb_timer` 状态机、**决死窗口的 bomb 救人分支**
  （M0-7 留的 stub）、`bombs` 账本、bomb 演出请求。bomb 是 Field 的首个租户，另开一刀。
- **消弹转分 / 掉星星道具**：属"账本公平性世界侧固定"，须与道具计价一起定 → 道具池那刀。
- **`reqs`（通道 B）**：field 的圈特效请求。属 M2 表现层题目。
- **敌人消自机弹**：需加矩阵行 8（Field × PlayerShot）= 扩矩阵 = 过评审。东方无此机制，YAGNI。
- **`owner` 语义**：待 co-op 记分 / 道具那刀。

## 权威设计回写（实施时一并做）

- `stg-world-design.md` D6：`BombFieldPool` → `FieldPool`（正名 + 能力位 + 常量）。
- `stg-world-design.md` D8 矩阵行 6/7：主动方 `BombField` → `Field`，注明按 `flags` 能力位启用；
  删去"已清除标记的弹不参与检测"（不可达，见偏离 3），保留 `delay > 0` 那半条。
- `stg-world-design.md` D9 趟一：逐弹 `BulletCleared` → 聚合 `FieldCleared`（附容量推导）。
- `stg-world-design.md` A5：events 生产者清单补 `FieldCleared`。
- `stg-world-design.md` D10：容量表加 `FieldPool` 16 × ~18B + generation/alive ≈ **320 B**
  （SoA：x/y/radius 3×4B + dmg_per_frame 2B + life 2B + owner 1B + flags 1B = 18B/槽）。

## 最终复审修正（2026-07-15）

上面 `FIELD_MAX_RADIUS` 的推导只证了一半：只钳 `field.radius` 这一侧，约束的是碰撞和里的主动
操作数；`Fx::Add` 是裸 `i32` 加法（debug panic / release 回绕），而被动半径当时完全无钳
（`create_bullet`/`create_enemy`/`create_player_shot` 均无上下界限制），一旦被动半径超过
~31744px，`field.radius + passive.radius` 依旧会溢出 `i32::MAX`（负被动半径同理会重现"和变负、
平方仍为正"的病）。当前调用方半径都 ≤16px 不可达，但 M1 的 ECL syscall 会把半径开放给脚本设置，
届时即可达。修正：改为共享常量 `MAX_ENTITY_RADIUS = 1024px`，在**每一个**带半径的写 API
（`create_bullet`/`create_enemy`（radius+hurtbox）/`create_player_shot`/`create_field`）上双边
钳制，使"和永不溢出"对六行碰撞的两个操作数都成立。

**第二轮复审**：上一版补齐的是池侧——四个写 API 覆盖了弹/敌/自机弹/field 的 radius/hurtbox，
但行 1/2/3 的被动操作数是 `player.hit_radius`/`graze_radius`，它们不在这四个 API 的覆盖范围内，
由 `PlayerState::spawn` 直接赋值。`WorldBody.players` 与 `PlayerState` 的字段都是 `pub`，
所以"这两个半径 ≤ MAX_ENTITY_RADIUS"是一条**前提**（除 spawn 外无人写它），不是写 API 强制出的
结论——与本次要修正的"半个证明当整个"是同一类问题，只是挪到了自机这一侧。现已在 `player.rs`
加编译期断言钉死 `HIT_RADIUS`/`GRAZE_RADIUS` 的上限，收紧 `players`/`PlayerState` 字段可见性
（或改走访问器）列为后续候选项，本次不做。
