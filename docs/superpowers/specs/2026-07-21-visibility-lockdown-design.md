# 可见性收口（刀 A）设计

> **一句话**：让自机（`players`）遵守池早就遵守的可见性纪律，并堵上池自己的逃生口
> （`define_pool!` 的 `alloc`/`free`），使"断层线以上只能走写 API / 读访问器"对**自机**与**池**
> 都成为**编译期强制**，而不再只是前提。收 follow-ups **D1 + D4** 两条设计裂缝。

**目标**：把 M2 表现层接入前必须焊死的可见性裂缝一次收齐——纯重构，金向量逐位不变。

**归属**：M2（stg-godot 表现层）**前置刀之一**。本刀只做"密封写口 + 最小只读种子"；不建完整
`WorldView`（通道 A），不建 `emit_req`/`reqs`（通道 B / anm call）——两者押后到 M2 开工时随
MultiMesh 桥一起定形。

---

## 1. 背景：为什么现在，收什么

表现层（godot / py / harness）持 `&mut World` 时的**写入面**目前有两个绕过写 API 的口子，均已记档：

- **D1**：`WorldBody.players` 字段与 `PlayerState` 内部字段全 `pub`。任何持 `&mut World` 的上层能
  `players[i].hit_radius = 30000` 直写，绕过 `WorldTables::validate()` 的半径钳制等一切契约。
  代码已自证此痛——`PlayerState::power_tier()` 特意 `min(4)` 防御，注释原话：*"`power` 是 pub 字段，
  导演/ECL 直写超 `POWER_MAX` 的非常规值时…release 下 index OOB"*。**字段可写 → 下游到处贴防御钳。**
- **D4**：`define_pool!` 生成的 `alloc`/`free` 是 `pub`。外部可 `bullets.free(handle)` 直接释放一颗
  挂着变换段的弹，跳过 `create_bullet_with_xform` 死亡路径里"弹死还段"那一步 → xform 段永久泄漏
  （非内存不安全，是段池慢性耗尽）。

**触发点**：M2 是第一个真正持有 `&mut World` 的外部消费者。趁它落地前把口封上，M2 接入近乎免费。

### 地面实况（断层线以上真正碰了什么）

`stg-harness` 是当前**唯一**的 out-of-crate `World` 消费者（`stg-ecl-compiler` 只产出 `EclImage`，
不跑世界）。逐点核对它碰的内部：

| 碰法 | 位置 | 现状 | 收口后 |
|---|---|---|---|
| **写** `players[0].power = 400/250` | main.rs 137 / 239 / 813 / 816（导演拔火力档，压满自机弹负载） | 直写 pub 字段 | 改走 `set_player_power(0, …)` |
| **读** `bullets/enemies/shots.iter_alive().count()`、`enemies.get(boss)` | main.rs 多处 | 走 pub 方法 | **不变**（方法保留 pub） |
| **写 API** `create_enemy` / `set_var` / `boss_ui`/`diag` 读 | main.rs 多处 | 已是写 API / pub 读 | **不变** |
| `.alloc(` / `.free(` 直调 | —— | **一处都没有**（补敌走 `create_enemy`） | 收 `pub(crate)` **零破坏** |

**结论**：收 `alloc`/`free` 为 `pub(crate)` 零破坏；收 `players` 为 `pub(crate)` 只崩那四处 `power=`
直写。D4 的"直接 free 漏段"那半——由**外部根本 free 不了**（编译期）**按构造消除**，无需新增 destroy API。

---

## 2. 设计：让 players 照抄池的纪律

池的纪律：**SoA 数组 `pub(crate)` 封死 → 写走 write API → 读走 `iter_alive`/`get`**。本刀让自机对称：

### 2.1 封写口

- `crates/stg-core/src/world.rs`：`WorldBody.players` 字段 `pub → pub(crate)`。
  - `PlayerState` 内部字段**保持 `pub`**：外部只能经只读访问器拿到 `&PlayerState`，`&`（非 `&mut`）
    只读不可写，数组已封 → 无 `&mut` 路径可达。**零字段级改动**即达成只读（省 20+ 字段逐个改的 churn）。
- `crates/stg-derive/src/lib.rs`（`define_pool!` proc-macro）：`pub fn alloc` → `pub(crate) fn alloc`、
  `pub fn free` → `pub(crate) fn free`。其余 `get`/`is_alive`/`iter_alive`/`copy_into` 保持 `pub`
  （纯读 + 快照原语，非写入面）；`free_index` 本已 `pub(crate)`，不动。

### 2.2 补写 API（自机唯一的合法外部写入）

- `crates/stg-core/src/world.rs`，`WorldBody` impl 写 API 区，新增：

  ```rust
  /// 拔火力档（导演/游戏层唯一的自机 power 外部写入口）。P4-b：越界 player 索引 → no-op +
  /// contract_viol 计数（同 SYS_SET_VAR 系统段守卫口径）；power 上钳 POWER_MAX（防 power_tier
  /// index OOB 的根，见 D1）。
  pub fn set_player_power(&mut self, player: usize, power: u16) {
      if player >= crate::MAX_PLAYERS {
          self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
          self.last_status = crate::world::STATUS_BAD_ARGS;
          return;
      }
      self.players[player].power = power.min(crate::items::POWER_MAX);
  }
  ```

  今天只有 `power` 有合法外部写需求（导演拔档）。**其余自机字段一律无外部写入口**——需要时再按同款
  P4-b 写 API 逐个开（YAGNI）。

### 2.3 补只读种子（通道 A 的最小种子，不雕完整视图）

- `WorldBody` 新增只读访问器：

  ```rust
  /// 自机只读切片（表现层读自机态的入口；通道 A 最小种子，非完整 WorldView）。
  pub fn players(&self) -> &[crate::player::PlayerState] {
      &self.players
  }
  ```

  字段名 `players` 与方法名 `players()` 在 Rust 里不冲突（成员 vs 方法）。当前无 out-of-crate 读者
  （harness 只写 power），此访问器纯为封口完整性 + godot 就绪；**不承诺 WorldView 的最终形状**。

### 2.4 harness 迁移

`crates/stg-harness/src/main.rs`：四处 `players[0].power = N`（`w.body.` 两处 + 导演闭包内 `b.` 两处）
→ `set_player_power(0, N)`。写入值 400/250 均 ≤ `POWER_MAX(400)`，钳为 no-op → **状态逐位不变**。

---

## 3. 不做什么（划界）

- **完整 `WorldView`（通道 A）**：只做 §2.3 的最小 `players()` 种子；完整读侧视图随 M2 MultiMesh 桥定形。
- **`emit_req` / `reqs` / `RenderReq`（通道 B / anm call）**：整条押后到下一刀。
- **destroy / kill 写 API**：外部 free 已被 §2.1 按构造封死，无消费者需要主动销毁实体（模拟自决生死）。
- **`PlayerState` 内部字段逐个 `pub(crate)`**：数组已封，`&` 下内部字段 `pub` 本就写不了 → 边际收益近零，
  且逐字段改会牵动大量 in-crate 读点，得不偿失。
- **删 `power_tier()` 的防御 `min(4)`**：留作 defense-in-depth（ECL/credit_item 等其他路径仍可能越界）。

---

## 4. 确定性与金向量论证

- **可见性变更不触碰运行期**：`pub → pub(crate)` 是编译期约束，不改任何字节演化。
- **`set_player_power` 逐位等价直写**：harness 写值 ≤ `POWER_MAX`，钳分支不触发，等同原 `players[0].power = N`。
- **无新依赖**：不引入 `trybuild` 等 dev-dep（避免搅动 `cargo tree -p stg-core` 依赖防火墙断言）。

⇒ 两段金向量（`golden`）逐帧校验和流**逐位不变**，收口前后 `diff` 应全等。

---

## 5. 测试策略

CLAUDE.md 铁律：招牌不变量必须有**判别式单测**（金向量守不了行为回归）。可见性是**编译期**性质，
其"值"由收口后 harness 必须改走访问器/写 API 才能编译来实现；行为面则由写 API 的守卫单测钉死。

1. **`set_player_power` 判别式单测**（stg-core）：
   - 钳边界：`set_player_power(0, POWER_MAX)` 写入恰 400；`set_player_power(0, POWER_MAX+1)` 写入
     被钳为 400（判别 `min` 是否真在，换成直写即红）。
   - P4-b 越界：`set_player_power(MAX_PLAYERS, 100)` → 无写入 + `contract_viol` +1 + `last_status=BAD_ARGS`。
2. **`players()` 访问器**：返回长度 `MAX_PLAYERS` 的切片，`players()[0]` 与内部一致（可折进既有测试）。
3. **金向量逐位不变**：`cargo run -p stg-harness -- golden --out after.txt`，与收口前基线 `diff` 全等
   （回归闸——证明纯重构未改任何值）。
4. **全绿 + 防火墙**：`cargo build/test --workspace`、`fmt --check`、`clippy -D warnings`、
   `cargo tree -p stg-core` 无新依赖。

---

## 6. 收尾（follow-ups 销账）

- `docs/follow-ups.md`：**D1 收口、D4 收口**——删两条（别留墓碑，git log 即历史）。
- `crates/stg-core/src/world.rs:69` 那段"`players` 目前都是 `pub`…"的裂缝注释：改写为"已收口"口径或删。
- `PROGRESS.md`：milestone 史加一行 + 重写「现在」段（若本刀作为独立收口刀合入）。
