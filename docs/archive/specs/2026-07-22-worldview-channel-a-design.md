# 通道 A / WorldView（读侧零拷贝视图）设计

> **一句话**：给子弹/敌人/自机弹/道具/作用区五池提供**只读零拷贝视图**——每池暴露 SoA 各字段裸切片
> `&[T]` + 存活位字 `&[u64]`，经 `WorldBody::view() -> WorldView` 单入口交出。落地 `stg-world-design.md`
> A9 早已拍板、刻意推到 M2 的 `PoolView` 契约，并顺手焊死 follow-up **D5**（池结构体字段整赋值残留）。

**目标**：为 M2 表现层（Godot MultiMesh 批量绘制）与 headless（观测张量）提供密集实体的读入口——纯重构，
金向量逐位不变；view 成为断层线以上唯一读路径，与刀 A「唯一写路径」对称。

**归属**：M2 表现层**前置读侧刀**（承接可见性收口刀 A）。

---

## 1. 背景：A9 早已拍板，现在是定契约的时刻

`stg-world-design.md` A9（§226-254，20 轮评审产物）已把通道 A 定死：

- 通道 A = **零拷贝类型化视图** `WorldView<'w>`，每池给 **SoA 各字段裸切片 + 存活位字 `&[u64]` + 活跃计数**。
- **死槽照常暴露，过滤是消费者义务**：给 `iter_alive()` 便利迭代器，但 MultiMesh/numpy 批量消费者直接拿
  裸切片 + 位掩码自己扫（吃满 SoA 带宽红利）。
- **有效期靠借用检查器**：`WorldView` 借 `&World`/`&WorldBody`，活着时 step（要 `&mut`）自然编不过。
- **布局半冻结**：SoA 顺序/对齐是跨语言契约（PyO3 numpy 零拷贝），改动过评审 + bump `engine_ver`。
- 设计**故意把 `PoolView` 推迟 M2**（§310）——"唯一消费者 Godot/PyO3，随其真实布局定半冻结契约"。**现在就是。**

**与刀 A 严丝合缝**：刀 A 把 SoA 数组 + `alloc`/`free` 收 `pub(crate)`，外部**读不到** `bullets.x[i]`——
本刀的裸切片访问器正是那把缺失的读钥匙；写口已封，读口现补齐，两刀合成完整的 crate 级读写纪律。

---

## 2. 设计

### 2.1 宏生成每池只读裸切片访问器（`stg-derive`）

`define_pool!`（`crates/stg-derive/src/lib.rs`）为每池**统一生成**：

```rust
impl #pool {
    #( pub fn #fnames(&self) -> &[#ftypes] { &self.#fnames } )*   // 每字段一个裸切片访问器
    pub fn alive_words(&self) -> &[u64] { &self.alive }          // 存活位字（批量消费者自己扫）
}
```

- **全字段暴露**（非渲染子集）：宏里一圈统一生成，零成本（未调用的访问器不占运行期）；headless RL 观测要全态；
  `#[repr(C)]` SoA 布局本就为 memcpy 快照/校验和冻结，暴露访问器**不新增冻结面**。
- 字段名 = 方法名（Rust 合法：`p.x` 字段 vs `p.x()` 方法）；in-crate 仍 `self.x[i]` 索引，外部只能 `p.x()`。
  现有字段名与既有方法（`new`/`alloc`/`get`/`free`/`is_alive`/`iter_alive`/`copy_into`/`cap`）**无一冲突**。
- `cap()`/`iter_alive()`/`get()`/`is_alive()` 保持 pub 不动（活跃计数/便利迭代/句柄解引）。

### 2.2 `WorldView` 结构 + `view()` 单入口

新增 `crates/stg-core/src/world/view.rs`（`mod view` + `pub use view::WorldView` 于 `world.rs`）：

```rust
pub struct WorldView<'w> { body: &'w WorldBody }
impl<'w> WorldView<'w> {
    pub fn bullets(self) -> &'w BulletPool { &self.body.bullets }
    pub fn shots(self)   -> &'w ShotPool   { &self.body.shots }
    pub fn enemies(self) -> &'w EnemyPool  { &self.body.enemies }
    pub fn items(self)   -> &'w ItemPool   { &self.body.items }
    pub fn fields(self)  -> &'w FieldPool  { &self.body.fields }
    pub fn players(self) -> &'w [crate::player::PlayerState] { self.body.players() } // 刀 A 已有
}
```
（`WorldView: Copy`，方法取 `self` 以还 `'w` 生命；`&Pool` 本身即只读视图——`alloc`/`free` 刀 A 已封 `pub(crate)`，
交出去外部只能读，**不必再套 `BulletView` wrapper**。）

- `WorldBody::view(&self) -> WorldView<'_>`（`world.rs`）：核心入口，闭包内 `b.view()`（`&mut` 重借 `&`）与
  step 后 `w.body.view()` 皆走它。
- `World::view(&self) -> WorldView<'_>`（`step.rs`）：委派 `self.body.view()`，godot 持 `World` 用。

### 2.3 焊 D5：封池结构体字段写口

`crates/stg-core/src/world.rs` 的 `WorldBody`：五个池**结构体字段** `pub → pub(crate)`——
`bullets` / `shots` / `enemies` / `fields` / `items`。封后外部再不能 `w.body.bullets = BulletPool::new()`
（D5 整赋值）或 `w.body.bullets.x[i]`（越 view 直读）；读一律经 `view()`。
（`boss_ui`/`globals` 非池、另有暴露形态，**不在本刀**；`players` 刀 A 已 `pub(crate)`；`xforms`/`signals` 本已封。）

### 2.4 harness 迁移（7 处池读经 view）

`crates/stg-harness/src/main.rs`，逐处 `.<pool>.<读>` → `.view().<pool>().<读>`：

| 行 | 现状 | 迁为 |
|---|---|---|
| 144 | `b.bullets.iter_alive().count()` | `b.view().bullets().iter_alive().count()` |
| 247 | `b.enemies.iter_alive().count()` | `b.view().enemies().iter_alive().count()` |
| 341 | `w.body.bullets.iter_alive().count()` | `w.body.view().bullets().iter_alive().count()` |
| 510 | `b.enemies.iter_alive().count()` | `b.view().enemies().iter_alive().count()` |
| 1074 | `w.body.bullets.iter_alive().count()` | `w.body.view().bullets().iter_alive().count()` |
| 1077 | `w.body.enemies.get(boss).is_some()` | `w.body.view().enemies().get(boss).is_some()` |
| 1143 | `w_disk.body.shots.iter_alive().count()` | `w_disk.body.view().shots().iter_alive().count()` |

（144/247/510 的 `b: &mut WorldBody` 是导演闭包参；`b.view()` 语句内即取即弃，不与后续可变用重叠。）

### 2.5 断层线：view 只出原始整数

`WorldView` 一律返回 `&[Fx]`/`&[Angle]`/`&[u16]` **原始定点/整数切片**，**绝不转 float**（float 在 stg-core =
I1 违规）。定点→浮点转换只发生在 Godot 桥（断层线上，design_doc §6.4）。numpy 零拷贝直接指向裸切片。

---

## 3. 不做什么（划界）

- **PyO3 `PoolView<Cols>` 泛型机器 / numpy dtype 描述子**：YAGNI——`#[repr(C)]` SoA 裸切片已是零拷贝契约，
  待 `stg-py`（M5）真存在再定 Cols 描述；本刀只出类型化裸切片。
- **`boss_ui` / `globals` 收口或纳入 view**：非池、已各有暴露形态（A9），与"密集实体"无关，另刀议。
- **通道 B / `emit_req` / `reqs`（anm call）**：独立刀，押后。
- **view 内做 float 转换 / 坐标变换**：违 I1，坚决不做（留 Godot 桥）。
- **curated 渲染子集**：否决——全字段统一生成更省代码、服务 RL 全态、不新增冻结面。
- **`BulletView`/`EnemyView` 包装层**：否决——`&Pool` 经刀 A 已是只读，多套一层无收益。

---

## 4. 确定性与金向量论证

- view 纯读（借 `&WorldBody`），不改任何字节演化；宏加的是未被模拟调用的只读访问器。
- 池字段 `pub→pub(crate)` 是编译期约束，零运行期影响；harness 迁移只换读路径（`iter_alive`/`get` 同一底层）。
- 零新依赖（`cargo tree -p stg-core` 防火墙不动；不引 trybuild）。

⇒ 两段金向量逐帧校验和流**逐位不变**，收口前后 `diff` 全等。

---

## 5. 测试策略

可见性/借用是**编译期**性质；行为面（裸切片指向真 SoA、位字反映存活）由判别式单测钉死。

1. **裸切片指向真 SoA**（stg-core）：建两颗已知 x 的弹，经 `view().bullets().x()` 取切片，按 `alive_words()`
   位扫读回，断言 x 值逐位命中（判别："x() 错返 y() 切片" / "返回错池" 即红）。
2. **`alive_words()` 反映位掩码**：建 N 颗、free 若干，断言 `alive_words` 的 popcount == 活跃数、且活跃 index 位置 1。
3. **裸切片+位掩码扫 ≡ iter_alive**：手扫 `alive_words` 得的 alive index 集合 == `iter_alive()` 集合
   （钉死批量路径与便利迭代一致——A9 "过滤是消费者义务" 的正确性）。
4. **金向量逐位不变**：`cargo run -q -p stg-harness -- golden` 前后 `diff` 全等（纯重构回归闸）。
5. **借用安全（编译期，仅记档不单测）**：`WorldView` 活着借 `&WorldBody`，同段 `step`（要 `&mut`）编不过——
   借用检查器天然保证（design_doc/A9），不引 trybuild 去断言编译失败（防火墙纪律）。
6. **全绿 + 防火墙**：`cargo build/test --workspace`、`fmt --check`、`clippy -D warnings`、`cargo tree -p stg-core` 无新依赖。

---

## 6. 半冻结契约记档

- **访问器名 + `#[repr(C)]` SoA 顺序即跨语言契约**：改字段名/顺序 = 破坏 Godot 桥与（将来）numpy 零拷贝，
  须过评审 + 视情 bump `engine_ver`（承接 A9 "布局半冻结"）。本刀不冻结新东西——SoA 布局本已为快照/校验和冻结。
- 待 `stg-py`（M5）真接 numpy 时，在此契约上加 Cols/dtype 描述（本刀不做）。

---

## 7. 收尾

- `docs/follow-ups.md`：**删 D5**（池字段整赋值残留——本刀焊死，无墓碑）。
- `docs/architecture.md`：M2 接缝行「WorldView 通道 A」标为已就位（读侧种子 → 完整 view）。
- `PROGRESS.md`：合入时 milestone 史加一行 + 重写「现在」段。
