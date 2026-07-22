# 通道 A / WorldView Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给五池（bullets/shots/enemies/items/fields）提供只读零拷贝视图——每池暴露 SoA 各字段裸切片 + 存活位字，经 `WorldBody::view() -> WorldView` 单入口交出；并把池结构体字段收 `pub(crate)` 焊死 follow-up D5。纯重构，金向量逐位不变。

**Architecture:** 三刀。① `define_pool!` 宏为每池统一生成 `pub fn <field>(&self) -> &[T]` 裸切片访问器 + `alive_words()`（additive，零破坏）；② 新增 `WorldView<'w>` 结构 + `WorldBody::view()` + `World::view()` 委派（additive）；③ 五池结构体字段 `pub→pub(crate)` 封写口 + harness 7 处池读迁 `view()` + 销 D5（同刀落地保持编译）。每刀结束全绿；刀 ③ 后金向量与刀 ① 前基线 `diff` 全等。

**Tech Stack:** Rust 1.92.0 / edition 2024；stg-core（确定性内核）、stg-derive（`define_pool!` proc-macro）、stg-harness（CLI 金向量）。

## Global Constraints

- **断层线纪律**：stg-core 不得引入 float / 时钟 / 宿主 RNG / 无序容器 / 任何新依赖。**零新依赖**（不引 `trybuild`），`cargo tree -p stg-core` 防火墙保持干净。
- **view 只出原始整数**：`WorldView` 一律返回 `&[Fx]`/`&[Angle]`/`&[u16]` 等**原始定点/整数切片**，**绝不转 float**（float 在 stg-core = I1 违规；转换留 Godot 桥）。
- **金向量逐位不变**：纯重构，`cargo run -q -p stg-harness -- golden` 每刀后（尤其刀 ③）与刀 ① 前基线 `diff` 全等。
- **只读视图**：`WorldView` 借 `&WorldBody`，不暴露任何 `&mut`；`alloc`/`free`（刀 A 已 `pub(crate)`）不因本刀重新开放。
- **提交结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- **不做**：PyO3 `PoolView<Cols>` 泛型/numpy dtype、`boss_ui`/`globals` 收口、通道 B/`emit_req`、`BulletView` 包装层、view 内 float 转换、curated 渲染子集。

---

## File Structure

| 文件 | 职责 | 刀 |
|---|---|---|
| `crates/stg-derive/src/lib.rs` | `define_pool!` 生成体加 `pub fn <field>(&self)->&[T]`（每字段）+ `alive_words()->&[u64]` | 1 |
| `crates/stg-core/src/bullets.rs` | `mod tests` 用 `Tp` 池加裸切片/位字/位扫≡iter_alive 判别式单测 | 1 |
| `crates/stg-core/src/world/view.rs`（**新建**） | `WorldView<'w>` 结构 + 五池访问器 + `players()` | 2 |
| `crates/stg-core/src/world.rs` | `mod view;` + `pub use view::WorldView;`；`WorldBody::view()`；五池字段 `pub→pub(crate)` | 2, 3 |
| `crates/stg-core/src/step.rs` | `World::view()` 委派 `self.body.view()` | 2 |
| `crates/stg-harness/src/main.rs` | 7 处 `.<pool>.<读>` → `.view().<pool>().<读>` | 3 |
| `docs/follow-ups.md` | 删 D5 | 3 |

**合入时（finishing-a-development-branch，非 TDD 任务）**：`docs/architecture.md` M2 前置行标 WorldView 已就位；`PROGRESS.md` 史加一行 + 重写「现在」段。

---

### Task 1: `define_pool!` 裸切片访问器 + 判别式单测

**Files:**
- Modify: `crates/stg-derive/src/lib.rs`（`define_pool` 宏生成体，`iter_alive`(~269)与 `copy_into`(~271) 之间插入）
- Test: `crates/stg-core/src/bullets.rs`（`mod tests`(~37) 内，用既有 `Tp` 池）

**Interfaces:**
- Produces（刀 2/3 依赖）：每个 `define_pool!` 池（`BulletPool`/`ShotPool`/`EnemyPool`/`ItemPool`/`FieldPool` + 测试池 `TpPool`）新得：
  - `pub fn <field>(&self) -> &[<FieldType>]`（每声明字段一个裸切片访问器，如 `BulletPool::x(&self) -> &[Fx]`）。
  - `pub fn alive_words(&self) -> &[u64]`（存活位字切片）。
  - 既有 `cap()`/`iter_alive()`/`get()`/`is_alive()` 不变。

- [ ] **Step 1: 抓金向量基线（任何改动前，pristine）**

Run:
```bash
cd /data/sunyunbo/www/stg-engine
cargo run -q -p stg-harness -- golden --out /tmp/view-golden-baseline.txt
wc -l /tmp/view-golden-baseline.txt
```
Expected: 生成基线（两段金向量逐帧校验和，非空）。刀 ③ 与它 `diff`。

- [ ] **Step 2: 写失败测试**（`crates/stg-core/src/bullets.rs` 的 `mod tests` 内，`Tp` 池定义之后追加）

```rust
/// 裸切片访问器（通道 A）判别式：切片指向真 SoA + alive_words 反映位掩码 + 位扫 ≡ iter_alive。
#[test]
fn view_slices_point_to_soa_and_alive_words_reflect_bitmap() {
    let mut p = TpPool::new();
    let h0 = p.alloc(TpInit { a: 10, b: 1 }).unwrap();
    let h1 = p.alloc(TpInit { a: 20, b: 2 }).unwrap();
    let h2 = p.alloc(TpInit { a: 30, b: 3 }).unwrap();
    p.free(h1); // 中间释放：位掩码留洞

    // 裸切片指向真 SoA（判别："a() 错返 b() 切片" 即红）
    let a = p.a();
    assert_eq!(a.len(), TpPool::CAP);
    assert_eq!(a[h0.index as usize], 10);
    assert_eq!(a[h2.index as usize], 30);

    // alive_words 反映位掩码：popcount == 活跃数(2)，h0/h2 位 1、h1 位 0
    let aw = p.alive_words();
    let popcount: u32 = aw.iter().map(|w| w.count_ones()).sum();
    assert_eq!(popcount, 2);
    let bit = |i: usize| (aw[i / 64] >> (i % 64)) & 1;
    assert_eq!(bit(h0.index as usize), 1);
    assert_eq!(bit(h1.index as usize), 0, "已 free 位为 0");
    assert_eq!(bit(h2.index as usize), 1);

    // 位扫 ≡ iter_alive（钉死批量路径与便利迭代一致——A9 "过滤是消费者义务"）
    let scanned: Vec<usize> = (0..TpPool::CAP).filter(|&i| bit(i) == 1).collect();
    let iterated: Vec<usize> = p.iter_alive().collect();
    assert_eq!(scanned, iterated);
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p stg-core --lib view_slices_point_to_soa 2>&1 | tail -20`
Expected: 编译失败——`no method named a`/`no method named alive_words`（访问器未生成）。

- [ ] **Step 4: 宏生成访问器**（`crates/stg-derive/src/lib.rs`，把 `copy_into` 的 doc 注释那行作为锚点，在其前插入）

old_string（唯一锚点）:
```rust
            /// 安全逐字段快照拷贝（每条 SoA 数组 copy_from_slice = memcpy，原地无临时量）。
```
new_string:
```rust
            /// 只读裸切片访问器（通道 A，A9）——批量消费者拿它 + `alive_words()` 自扫存活。
            #(
                pub fn #fnames(&self) -> &[#ftypes] {
                    &self.#fnames
                }
            )*

            /// 存活位字切片（通道 A）——批量消费者按位扫活跃 index（`iter_alive` 的裸形态）。
            pub fn alive_words(&self) -> &[u64] {
                &self.alive
            }

            /// 安全逐字段快照拷贝（每条 SoA 数组 copy_from_slice = memcpy，原地无临时量）。
```
（`fnames`/`ftypes` 是宏顶部已备的 `Vec<&Ident>`/`Vec<&Type>`，此处复用同一重复展开；字段名 = 方法名 Rust 合法，in-crate 仍 `self.#fnames[idx]` 索引不受影响。`&self.#fnames` 是 `&[T; CAP]`，返回位置 unsized 强转 `&[T]`。）

- [ ] **Step 5: 跑测试确认通过 + 全 crate 绿**

Run: `cargo test -p stg-core --lib 2>&1 | tail -15`
Expected: 新测试 PASS，stg-core 全绿。

- [ ] **Step 6: fmt + clippy + 提交**

```bash
cd /data/sunyunbo/www/stg-engine
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
git add crates/stg-derive/src/lib.rs crates/stg-core/src/bullets.rs
git commit -m "$(cat <<'EOF'
feat(derive): define_pool! 每字段裸切片访问器 + alive_words（通道 A/1）

宏为每池统一生成 pub fn <field>(&self)->&[T] + alive_words()->&[u64]，供通道 A
批量消费者拿裸切片 + 位字自扫（A9）。additive，零破坏。Tp 池判别式单测钉切片指向真
SoA + 位字反映掩码 + 位扫≡iter_alive。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `WorldView` 结构 + `view()` 入口

**Files:**
- Create: `crates/stg-core/src/world/view.rs`
- Modify: `crates/stg-core/src/world.rs`（`mod view;` + `pub use`；`WorldBody::view()`）
- Modify: `crates/stg-core/src/step.rs`（`World::view()` 委派）
- Test: `crates/stg-core/src/world.rs`（`mod tests`(~816)）

**Interfaces:**
- Consumes：Task 1 的 `BulletPool::x()`/`iter_alive()` 等访问器。
- Produces（刀 3 依赖）：
  - `pub struct WorldView<'w>`（`stg_core::world::WorldView`，`Copy`）。
  - `WorldBody::view(&self) -> WorldView<'_>`；`World::view(&self) -> WorldView<'_>`（委派）。
  - `WorldView` 方法（取 `self`）：`bullets()/shots()/enemies()/items()/fields() -> &'w <Pool>`、`players() -> &'w [PlayerState]`。

> 本刀不动池字段可见性（仍 `pub`），故新增结构 + 方法后零破坏，独立可测。

- [ ] **Step 1: 写失败测试**（`crates/stg-core/src/world.rs` 的 `mod tests` 末尾追加）

```rust
/// WorldView 管线：view() 正确接线各池 + players 委派；World::view 与 WorldBody::view 同源。
#[test]
fn view_exposes_pools_and_players() {
    let mut w = crate::step::World::new(1);
    let _b = crate::world::test_support::bullet_at(&mut w, 5, 7);
    let _e = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 3);

    let vb = w.body.view();
    assert_eq!(vb.bullets().iter_alive().count(), 1);
    assert_eq!(vb.enemies().iter_alive().count(), 1);
    // 裸切片指向该弹 x（判别 bullets() 未错接 enemies()）
    let bi = vb.bullets().iter_alive().next().unwrap();
    assert_eq!(vb.bullets().x()[bi], Fx::from_int(5));
    // players() 委派刀 A 访问器
    assert_eq!(vb.players().len(), crate::MAX_PLAYERS);
    // World::view 委派 == WorldBody::view
    assert_eq!(w.view().bullets().iter_alive().count(), 1);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib view_exposes_pools 2>&1 | tail -20`
Expected: 编译失败——`no method named view`（`WorldBody::view`/`World::view` 未实现）。

- [ ] **Step 3: 新建 `crates/stg-core/src/world/view.rs`**

```rust
//! 通道 A 读侧零拷贝视图（A9）——step 后表现层/headless 经 `world.view()` 只读五池 SoA。
//! 借 `&WorldBody`：视图活着期间无法 step（要 `&mut`），借用检查器天然保证时序安全。
//! 只出原始定点/整数切片，绝不转 float（float 留 Godot 桥，守 I1）。

use super::WorldBody;
use crate::bullets::BulletPool;
use crate::enemy::EnemyPool;
use crate::field::FieldPool;
use crate::items::ItemPool;
use crate::player::PlayerState;
use crate::shots::ShotPool;

/// 只读世界视图。`Copy`（仅一个借用）；方法取 `self` 以还 `'w` 生命。
#[derive(Clone, Copy)]
pub struct WorldView<'w> {
    pub(crate) body: &'w WorldBody,
}

impl<'w> WorldView<'w> {
    pub fn bullets(self) -> &'w BulletPool {
        &self.body.bullets
    }
    pub fn shots(self) -> &'w ShotPool {
        &self.body.shots
    }
    pub fn enemies(self) -> &'w EnemyPool {
        &self.body.enemies
    }
    pub fn items(self) -> &'w ItemPool {
        &self.body.items
    }
    pub fn fields(self) -> &'w FieldPool {
        &self.body.fields
    }
    pub fn players(self) -> &'w [PlayerState] {
        self.body.players()
    }
}
```

- [ ] **Step 4: 在 `world.rs` 挂 `mod view` + `WorldBody::view()`**

4a. `crates/stg-core/src/world.rs` 模块声明区（`mod transform;`(~37) 之后）加：
```rust
mod view;

pub use view::WorldView;
```

4b. `impl WorldBody` 内（`players()` 访问器之后，刀 A 加的那个）加：
```rust
/// 通道 A 只读视图入口（A9）——step 后/相位间经它读五池 SoA；活着期间借 &self 挡住 step。
pub fn view(&self) -> WorldView<'_> {
    WorldView { body: self }
}
```

- [ ] **Step 5: 在 `step.rs` 加 `World::view()` 委派**（`impl World`(~34)，`seed()`(~108) 附近）

```rust
/// 通道 A 只读视图（委派 `WorldBody::view`）——godot/表现层持 `World`，经它读世界状态。
pub fn view(&self) -> crate::world::WorldView<'_> {
    self.body.view()
}
```

- [ ] **Step 6: 跑测试确认通过 + 全 crate 绿**

Run: `cargo test -p stg-core --lib 2>&1 | tail -15`
Expected: `view_exposes_pools_and_players` PASS，stg-core 全绿。

- [ ] **Step 7: fmt + clippy + 提交**

```bash
cd /data/sunyunbo/www/stg-engine
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
git add crates/stg-core/src/world/view.rs crates/stg-core/src/world.rs crates/stg-core/src/step.rs
git commit -m "$(cat <<'EOF'
feat(world): WorldView 结构 + WorldBody::view()/World::view()（通道 A/2）

新增 world/view.rs：WorldView<'w> 借 &WorldBody，五池访问器 + players() 委派；只出
原始定点切片（float 留 Godot 桥）。view() 活着借 &self 挡 step（借用检查器保时序）。
additive 零破坏。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 焊 D5——封五池写口 + harness 迁移 + 销 D5

**Files:**
- Modify: `crates/stg-core/src/world.rs`（`WorldBody` 五池字段 `pub→pub(crate)`：行 140/144/145/146/148）
- Modify: `crates/stg-harness/src/main.rs`（7 处池读迁 `view()`）
- Modify: `docs/follow-ups.md`（删 D5）

**Interfaces:**
- Consumes：Task 2 的 `WorldBody::view()`/`WorldView` 池访问器 + 既有 `iter_alive()`/`get()`。

> 封字段与 harness 迁移必须同刀落地，否则 harness 编不过。先迁 harness，再翻 `pub(crate)`。

- [ ] **Step 1: 迁移 harness 7 处池读**（`crates/stg-harness/src/main.rs`，逐处精确替换）

五处**唯一**子串（各一次 Edit）：
- 行 144：`let alive = b.bullets.iter_alive().count();` → `let alive = b.view().bullets().iter_alive().count();`
- 行 341：`w.body.bullets.iter_alive().count(),` → `w.body.view().bullets().iter_alive().count(),`
- 行 1074：`let bullet_count = w.body.bullets.iter_alive().count();` → `let bullet_count = w.body.view().bullets().iter_alive().count();`
- 行 1077：`w.body.enemies.get(boss).is_some(),` → `w.body.view().enemies().get(boss).is_some(),`
- 行 1143：`let shot_count = w_disk.body.shots.iter_alive().count();` → `let shot_count = w_disk.body.view().shots().iter_alive().count();`

一处**重复**子串（行 247 与 510——整行缩进不同但代码子串逐字相同，用 **`replace_all: true`** 一次替换两处）：
- `let alive = b.enemies.iter_alive().count();` → `let alive = b.view().enemies().iter_alive().count();`

（`b: &mut WorldBody` 是导演闭包参，`b.view()` 语句内即取即弃，不与后续可变用重叠。子串不含行首缩进，故对两处不同缩进的行均命中。）

- [ ] **Step 2: 封五池结构体字段写口**（`crates/stg-core/src/world.rs` 的 `WorldBody`）

逐字段 `pub → pub(crate)`：
- 行 140：`pub bullets: BulletPool,` → `pub(crate) bullets: BulletPool,`
- 行 144：`pub shots: ShotPool,` → `pub(crate) shots: ShotPool,`
- 行 145：`pub enemies: EnemyPool,` → `pub(crate) enemies: EnemyPool,`
- 行 146：`pub fields: FieldPool,` → `pub(crate) fields: FieldPool,`
- 行 148：`pub items: crate::items::ItemPool,` → `pub(crate) items: crate::items::ItemPool,`

（`boss_ui`/`globals` 非池、另有暴露形态，不动；`players` 刀 A 已 `pub(crate)`；`xforms`/`signals` 本已封。）

- [ ] **Step 3: 全工作区编译 + 测试绿**

Run: `cargo build --workspace 2>&1 | tail -5 && cargo test --workspace 2>&1 | tail -20`
Expected: 全绿。（若 harness 有遗漏的 `.<pool>.` 直读未迁 → 此处私有字段 `E0616` 会暴露。）

- [ ] **Step 4: 金向量与基线 diff 全等（纯重构回归闸）**

Run:
```bash
cargo run -q -p stg-harness -- golden --out /tmp/view-golden-after3.txt
diff /tmp/view-golden-baseline.txt /tmp/view-golden-after3.txt && echo "IDENTICAL"
```
Expected: 无 diff 输出 + `IDENTICAL`（迁移只换读路径、封字段是编译期变更 → 逐位不变）。

- [ ] **Step 5: 销 follow-up D5**（`docs/follow-ups.md`）

删整节 `### D5. 池结构体字段仍 `pub` …`（连同正文，到下一 `---`/`##` 前）。别留墓碑。

- [ ] **Step 6: fmt + clippy + 依赖防火墙 + 提交**

```bash
cd /data/sunyunbo/www/stg-engine
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
cargo tree -p stg-core 2>&1 | grep -iE "godot|libm|rand|chrono|time" && echo "FIREWALL BREACH" || echo "FIREWALL OK"
git add crates/stg-core/src/world.rs crates/stg-harness/src/main.rs docs/follow-ups.md
git commit -m "$(cat <<'EOF'
refactor(world): 封五池写口为 pub(crate)，读经 view（通道 A/3，焊 D5）

WorldBody 五池字段 pub→pub(crate)：外部再不能整池重赋值（D5）或越 view 直读 SoA；
读一律经 world.view()。harness 7 处池读迁 view()。金向量与基线逐位全等。删 follow-up D5。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Definition of Done（合入前）

- [ ] 三刀提交齐全，`cargo build/test --workspace` 全绿。
- [ ] `cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零告警。
- [ ] 金向量刀 ③ 后与基线 `diff` 全等（`/tmp/view-golden-baseline.txt`）。
- [ ] `cargo tree -p stg-core` 无 godot/libm/rand/时钟等禁项（零新依赖）。
- [ ] `docs/follow-ups.md` 中 D5 已删（无墓碑）。
- [ ] 合入时（finishing）：`docs/architecture.md` M2 前置行标 WorldView 已就位；`PROGRESS.md` 史加一行 + 重写「现在」段。
