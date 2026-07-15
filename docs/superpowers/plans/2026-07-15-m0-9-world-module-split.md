# M0-9 world.rs 模块拆分 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 1593 行的 `world.rs`（生产 ~820 + 测试 ~773）按**相位骨架**拆成 6 个文件，**零行为改动**。

**Architecture:** 模块结构镜像 step 的相位骨架（P2：相位顺序是宪法）。方法全是 `impl WorldBody`，同 crate 内拆到多个文件写多个 `impl` 块——**字段所有权不变、`step.rs` 的宪法顺序一行不动、写 API 边界不变**。`src/world.rs` 保留"字段所有权 + 跨相位共用设施"，每个有分量的相位各得一个 `src/world/*.rs`。

**Tech Stack:** Rust 2024（`world.rs` + `world/` 子模块的 2018+ 路径风格，不用 `mod.rs`）。

## Global Constraints

- **这是纯搬家。只搬不改。** 不重命名方法、不调整逻辑、不"顺手优化"、不补测试、不改可见性（除下方明列的常量提升）。任何行为改动都会让本计划的验收条件失效。
- **验收铁条：金向量校验和逐字节不变。** 每一刀之后 `stg-harness golden` 的 600 帧流必须与重构前的基线**完全相同**。这既是"没搬错"的证据，也是"没趁机改行为"的证据。
- **测试数量不变：103。** 每一刀之后 `cargo test --workspace` 必须仍是 103 passed——防止拆分时静默丢测试。
- **I7 布局 / P2 step 所有权 / P1 写 API 边界 / P6 全量校验** 均不受影响（不动字段、不动相位序、不动 API 签名）。
- 每刀必须过：`cargo test --workspace`（103）+ `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings`（零警告）。

## 目标结构

```
src/world.rs            常量 · DiagCounters · WorldBody · phase_enter · clamp_radius
                        · 4 个 create_* 写 API · push_hit/push_event
                        · begin(相0) · run_transforms(相4 stub) · advance(相10)
                        · #[cfg(test)] pub(crate) mod test_support
src/world/collide.rs    相位 6   collide + collide_bullets_player/body_player/shot_enemy
                                 /field_bullet/field_enemy
src/world/settle.rs     相位 7   settle + trigger_player_hit + damage_enemy
src/world/player.rs     相位 1+3 decode_input + update_players + commit_death
                                 + move_player + char0_update_shot
src/world/integrate.rs  相位 5   integrate
src/world/cleanup.rs    相位 9   cleanup + out_of_bounds
```

`crate::player` = `PlayerState` 数据模块（`bullets.rs`/`enemy.rs`/`field.rs`/`shots.rs` 的同辈）；
`crate::world::player` = 自机相位逻辑。路径本身区分二者，两个模块并存是有意的。

## 拆分机制（每刀通用，先在此说清）

- `src/world.rs` 顶部加 `mod xxx;`（子模块**不需要** `pub`——`impl WorldBody` 的方法可见性由方法自己的
  `pub(crate)`/私有 决定，与 impl 块所在模块无关）。
- 子文件顶部 `use super::WorldBody;` + 该文件真正用到的 `use`（**只搬需要的，别整块复制 world.rs 的 use 头**）。
- **私有方法必须和它唯一的调用者同处一个文件**（私有 = 仅定义模块及其后代可见）：
  `collide_*` 只被 `collide` 调 → 同去 collide.rs；`trigger_player_hit`/`damage_enemy` 只被 `settle` 调 → 同去 settle.rs；
  `commit_death`/`move_player`/`char0_update_shot` 只被 `update_players` 调 → 同去 player.rs；
  `out_of_bounds` 只被 `cleanup` 调 → 同去 cleanup.rs。
- **`pub(crate)` 方法**（`collide`/`settle`/`update_players`/`decode_input`/`integrate`/`cleanup`）搬家后
  仍可被 `step.rs` 调用，可见性不用改。
- `WorldBody` 的字段是 `pub`/`pub(crate)`，`phase_enter`/`push_hit`/`push_event` 是 `pub(crate)`
  → 子模块直接用，不用改。

## 共享常量提升（T3 时做）

`FIELD_HALF_W` / `FIELD_HEIGHT` / `OOB_MARGIN` 现为 `world.rs` 私有 const。拆分后
`move_player`（→ player.rs 的场界钳制）与 `out_of_bounds`（→ cleanup.rs）都要用
→ 提升为 `pub(crate)`，**留在 `world.rs`**（它们是世界的场界定义，不属于任何单一相位）。

## 测试搬迁规则

**规则：测试跟着它的被测对象走。** 跨相位的 step 级集成测试留 `world.rs`。

| 测试 | 去向 |
|---|---|
| `collide_*`（8 个） | `world/collide.rs` |
| `settle_*`（8 个） | `world/settle.rs` |
| `oob_detects_margin` | `world/cleanup.rs` |
| `hits_push_clear_and_overflow` / `events_push_records_fact` | `world.rs`（`push_*` 在此） |
| `create_*_clamps_*`（5 个） | `world.rs`（写 API 在此） |
| `create_enemy_and_integrate_moves` / `field_life_one_lives_exactly_one_frame` / `field_life_n_survives_n_frames` | `world.rs`（走 `step` 的跨相位集成测试） |

共用助手 `bullet_at` / `spawn_enemy` / `spawn_field` → `world.rs` 的
`#[cfg(test)] pub(crate) mod test_support`，各模块 `use crate::world::test_support::*;`。
**不许各文件复制一份**（逐字重复会被复审打回）。

---

## Task 0（前置，T1 的第一步）：钉下金向量基线

在动任何代码前，把重构前的校验和流固化下来——后面每一刀都拿它对。

```powershell
cargo run -p stg-harness -- golden --out $env:TEMP\baseline.txt
(Get-FileHash $env:TEMP\baseline.txt -Algorithm SHA256).Hash
```

记下这个 SHA-256。**每刀之后重跑 golden 并比对，必须完全一致。**

---

## Task 1: 抽出 `world/collide.rs`（相位 6）

**Files:**
- Create: `crates/stg-core/src/world/collide.rs`
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `world::collide` 子模块，内含 `impl WorldBody` 的 `pub(crate) fn collide` + 5 个私有 `collide_*`。
- Consumes: `WorldBody` 的字段与 `push_hit`（均已 `pub(crate)`/`pub`，无需改可见性）。

- [ ] **Step 1: 钉基线**

Run:
```powershell
cargo run -p stg-harness -- golden --out $env:TEMP\baseline.txt
(Get-FileHash $env:TEMP\baseline.txt -Algorithm SHA256).Hash
```
记下 SHA-256。这是后续每一刀的对照基准。

- [ ] **Step 2: 建 `world/collide.rs`，整块搬入**

Create `crates/stg-core/src/world/collide.rs`，文件头：
```rust
//! 相位 6 · 碰撞收集（D8 矩阵）。
//!
//! **只收集不改状态硬规则**：本相位纯读，只经 `push_hit` 追加 `hits`；改状态一律在 settle（相位 7）。
//! 半径映射见 D8：行1/2 弹×自机(hit/graze)、行3 敌体×自机(体碰 radius)、行4 自机弹×敌人(受击 hurtbox)、
//! 行6 作用区×敌弹、行7 作用区×敌人(受击 hurtbox)。

use super::WorldBody;
use crate::enemy::ENEMY_DYING;   // 若搬入的代码实际未用到则删掉此行
use crate::events::{
    ROW_BODY_PLAYER_HIT, ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT, ROW_FIELD_BULLET,
    ROW_FIELD_ENEMY, ROW_SHOT_ENEMY,
};
use crate::field::{FIELD_CLEAR_BULLETS, FIELD_DAMAGE};
use crate::math::geom::len_sq;

impl WorldBody {
    // ← 从 world.rs 原样剪切：collide / collide_bullets_player / collide_body_player
    //    / collide_shot_enemy / collide_field_bullet / collide_field_enemy
}
```

从 `world.rs` **剪切**（不是复制）这 6 个方法，原样贴入上面的 `impl` 块。方法体一个字符都不要改。
原方法内部若有 `use` 语句（如 `use crate::events::ROW_...;` 写在 fn 内），可保留原样，也可提到文件头
——**二选一，保持一致**；若提到文件头，务必确认没有未使用的 import（clippy 会报）。

- [ ] **Step 3: `world.rs` 声明子模块**

Modify `crates/stg-core/src/world.rs`，在 use 段之后、常量段之前加：
```rust
mod cleanup;
mod collide;
mod integrate;
mod player;
mod settle;
```
> **注**：本刀只创建 `collide.rs`，其余四个文件尚不存在——**本刀只写 `mod collide;` 一行**，其余四行在
> 各自的刀里追加。

清理 `world.rs` 顶部因搬走代码而变得未使用的 `use`（clippy 会精确指出）。

- [ ] **Step 4: 测试搬迁**

把 `world.rs` 测试模块里的 8 个 `collide_*` 测试剪切到 `world/collide.rs` 的
`#[cfg(test)] mod tests`。它们用到的助手 `bullet_at`/`spawn_enemy`/`spawn_field` 此刻仍在 `world.rs`
的 `mod tests` 里、跨模块不可见 —— **本刀先把这三个助手提为 `world.rs` 的
`#[cfg(test)] pub(crate) mod test_support`**（T4 原计划做，提前到此，因为 T1 就需要），
`world.rs` 自己的测试改为 `use crate::world::test_support::*;`，`collide.rs` 的测试同样。

`world/collide.rs` 测试模块头：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Fx;
    use crate::world::test_support::*;
    use crate::world::PH_COLLIDE;
    // ← 8 个 collide_* 测试原样贴入
}
```

- [ ] **Step 5: 验收——校验和 + 测试数**

Run:
```powershell
cargo test --workspace 2>&1 | Select-String "test result: ok"
cargo run -p stg-harness -- golden --out $env:TEMP\after1.txt
(Get-FileHash $env:TEMP\after1.txt -Algorithm SHA256).Hash
fc.exe $env:TEMP\baseline.txt $env:TEMP\after1.txt
```
Expected: **103 passed**；SHA-256 与基线**完全相同**；`fc` 报 `no differences encountered`。
**若校验和变了 → STOP，报告 BLOCKED**（说明这一刀改了行为，不是纯搬家）。

- [ ] **Step 6: fmt + clippy**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 零警告（特别注意搬家后 `world.rs` 残留的未使用 import）。

- [ ] **Step 7: Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/world/collide.rs
git commit -m "refactor(world): 抽出 world/collide.rs（相位6）+ test_support 共用助手

纯搬家：金向量校验和逐字节不变、103 测试不变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: 抽出 `world/settle.rs`（相位 7）

**Files:**
- Create: `crates/stg-core/src/world/settle.rs`
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `world::settle` 子模块，内含 `pub(crate) fn settle` + 私有 `trigger_player_hit` / `damage_enemy`。
- Consumes: T1 建立的 `test_support`；`WorldBody` 字段；`push_event`。

- [ ] **Step 1: 建 `world/settle.rs`，整块搬入**

Create `crates/stg-core/src/world/settle.rs`，文件头：
```rust
//! 相位 7 · 结算三趟（D9）——**唯一改状态者**。
//!
//! 趟一 清除/防护：行6 消弹（**标记不回收**，回收在 cleanup 相位9）+ 按 field 索引升序发聚合
//!   `FieldCleared`。**先于趟二** —— 故同帧作用区能救下本会命中自机的弹（bomb 救命）。
//! 趟二 伤害：行4/7 敌人扣血（overkill/无敌帧门禁）；行1/3 自机中弹 → 决死窗口（行1 跳过已清除的弹）。
//! 趟三 计分：graze（`grazed_by` 逐弹一次；**不查已清除位** —— 擦在相位6 已发生、清弹是相位7 的事）。

use super::WorldBody;
use crate::bullets::BULLET_CLEARED;
use crate::enemy::ENEMY_DYING;
use crate::events::Event;
use crate::field::FieldPool;

impl WorldBody {
    // ← 从 world.rs 原样剪切：trigger_player_hit / damage_enemy / settle
}
```
（实际 `use` 以搬入代码真正用到的为准——`settle` 里若用的是 `crate::events::ROW_*` 全限定路径就不必 import。）

从 `world.rs` **剪切**这 3 个方法，原样贴入。方法体一字不改（含 `const _: () = assert!(MAX_PLAYERS <= 8, ...)`
那句——它是 M0-7 最终复审要求加的位宽护栏，连同其解释注释一并搬走）。

- [ ] **Step 2: `world.rs` 加 `mod settle;`** + 清理未使用 import。

- [ ] **Step 3: 测试搬迁**

8 个 `settle_*` 测试剪切到 `world/settle.rs` 的 `#[cfg(test)] mod tests`，
`use crate::world::test_support::*;` + `use crate::world::PH_COLLIDE;`（settle 测试先调 `collide()` 再调
`settle()`，故仍需置 `phase_guard = PH_COLLIDE`）。

- [ ] **Step 4: 验收**

Run:
```powershell
cargo test --workspace 2>&1 | Select-String "test result: ok"
cargo run -p stg-harness -- golden --out $env:TEMP\after2.txt
fc.exe $env:TEMP\baseline.txt $env:TEMP\after2.txt
```
Expected: **103 passed**；`fc` 报无差异。校验和变了 → **STOP, BLOCKED**。

- [ ] **Step 5: fmt + clippy** — `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`，零警告。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/world/settle.rs
git commit -m "refactor(world): 抽出 world/settle.rs（相位7 结算三趟）

纯搬家：金向量校验和逐字节不变、103 测试不变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: 抽出 `world/player.rs`（相位 1+3）+ 场界常量提升

**Files:**
- Create: `crates/stg-core/src/world/player.rs`
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `world::player` 子模块，内含 `pub(crate) fn decode_input` / `pub(crate) fn update_players`
  + 私有 `commit_death` / `move_player` / `char0_update_shot`。
- Produces: `world::{FIELD_HALF_W, FIELD_HEIGHT, OOB_MARGIN}` 提升为 `pub(crate)`。
- Consumes: T1 的 `test_support`。

> **命名说明**（写进文件头注释）：`crate::player` = `PlayerState` **数据**模块（`bullets`/`enemy`/`field`/`shots`
> 的同辈）；`crate::world::player` = 自机**相位逻辑**。两者并存是有意的，路径本身区分。

- [ ] **Step 1: 场界常量提升为 `pub(crate)`**

Modify `crates/stg-core/src/world.rs`：
```rust
pub(crate) const FIELD_HALF_W: i32 = 192; // x ∈ [-192, 192]
pub(crate) const FIELD_HEIGHT: i32 = 448; // y ∈ [0, 448]
pub(crate) const OOB_MARGIN: i32 = 64; // 越界回收边距
```
（原为私有 const；`move_player` 的场界钳制与 `out_of_bounds` 拆到不同文件后都要用。它们是**世界的场界
定义**，不属任何单一相位，故留在 `world.rs`。）

- [ ] **Step 2: 建 `world/player.rs`，整块搬入**

Create `crates/stg-core/src/world/player.rs`，文件头：
```rust
//! 相位 1 · 输入译码 + 相位 3 · 自机更新（D6/A8）。
//!
//! **命名**：`crate::player` 是 `PlayerState` 数据模块（`bullets`/`enemy`/`field`/`shots` 的同辈）；
//! 本模块是自机的**相位逻辑**。二者并存，路径区分。
//!
//! **生死状态机的触发与计时分家**：本相位（3）独占**全部计时**（决死窗口倒数 → `commit_death`
//! → 重生 → 无敌耗尽 → Alive）；**中弹触发**（Alive → DeathWindow）在 settle（相位 7）。
//! 因相位 3 早于 7，中弹在帧尾定、窗口从次帧起数 —— 这 1 帧错位正是决死窗口的语义。

use super::WorldBody;
use crate::events::Event;
use crate::math::Fx;
use crate::shots::ShotInit;

impl WorldBody {
    // ← 从 world.rs 原样剪切：decode_input / update_players / commit_death
    //    / move_player / char0_update_shot
}
```
（`use` 以实际用到的为准。`move_player` 里的 `FIELD_HALF_W`/`FIELD_HEIGHT` 改为 `super::FIELD_HALF_W`
等，或在文件头 `use super::{FIELD_HALF_W, FIELD_HEIGHT};`——**除此之外方法体一字不改**。
`char0_update_shot` 里的 `match character_id` 是 A8 角色分发点，**原样搬走，本刀不动**。）

- [ ] **Step 3: `world.rs` 加 `mod player;`** + 清理未使用 import。

- [ ] **Step 4: 测试搬迁**

`world.rs` 测试模块里没有专属的 player 测试（自机行为测试住在 `step.rs`，本刀不动 `step.rs`）。
若确有 player 相关测试在 `world.rs`，一并搬入 `world/player.rs`。**核对：搬完总数仍是 103。**

- [ ] **Step 5: 验收**

Run:
```powershell
cargo test --workspace 2>&1 | Select-String "test result: ok"
cargo run -p stg-harness -- golden --out $env:TEMP\after3.txt
fc.exe $env:TEMP\baseline.txt $env:TEMP\after3.txt
```
Expected: **103 passed**；`fc` 报无差异。校验和变了 → **STOP, BLOCKED**。

- [ ] **Step 6: fmt + clippy** — 零警告。

- [ ] **Step 7: Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/world/player.rs
git commit -m "refactor(world): 抽出 world/player.rs（相位1 译码 + 相位3 自机更新）+ 场界常量提 pub(crate)

纯搬家：金向量校验和逐字节不变、103 测试不变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: 抽出 `world/integrate.rs`（相位 5）+ `world/cleanup.rs`（相位 9）+ 收口

**Files:**
- Create: `crates/stg-core/src/world/integrate.rs`, `crates/stg-core/src/world/cleanup.rs`
- Modify: `crates/stg-core/src/world.rs`

**Interfaces:**
- Produces: `world::integrate`（`pub(crate) fn integrate`）；`world::cleanup`（`pub(crate) fn cleanup` + 私有 `out_of_bounds`）。

- [ ] **Step 1: 建 `world/integrate.rs`**

Create，文件头：
```rust
//! 相位 5 · 积分（各池 `pos += vel` + 计时器倒数）。
//!
//! 弹/自机弹/敌人：`pos += vel`（弹另有 `delay` 门与 `life` 倒数；敌人另 tick `invuln`/`hit_flash`；
//! 敌人的 `move_to` 插值器待后续切片，`mv_*` 字段现为惰性）。
//! 作用区：`life` 倒数 —— `life=1` 本帧减到 0、相位 6 仍参与判定、相位 9 才回收（"每帧重铺=跟随"的时序基础）。

use super::WorldBody;
```
从 `world.rs` 剪切 `integrate`，原样贴入。

- [ ] **Step 2: 建 `world/cleanup.rs`**

Create，文件头：
```rust
//! 相位 9 · 回收（越界 / 寿命尽 / 已清除弹 / dying 敌人 / 寿命尽作用区）。
//!
//! **敌人与已清除弹在此收尸**：settle（相位 7）只打标记，槽要活过相位 8（ECL 挂钩）供死亡脚本
//! 与表现层读取死亡坐标，故回收统一延到本相位。

use super::WorldBody;
use crate::bullets::BULLET_CLEARED;
use crate::enemy::ENEMY_DYING;
use crate::math::Fx;
use super::{FIELD_HALF_W, FIELD_HEIGHT, OOB_MARGIN};
```
从 `world.rs` 剪切 `cleanup` + `out_of_bounds`，原样贴入。`out_of_bounds` 里的场界常量改用
`super::` 路径（或文件头 import）——**除此之外一字不改**。

- [ ] **Step 3: `world.rs` 加 `mod integrate; mod cleanup;`** + 清理未使用 import。

此时 `world.rs` 顶部的模块声明应齐全：
```rust
mod cleanup;
mod collide;
mod integrate;
mod player;
mod settle;
```

- [ ] **Step 4: 测试搬迁**

`oob_detects_margin` → `world/cleanup.rs`。其余（`hits_push_*`/`events_push_*`/`create_*_clamps_*`/
`create_enemy_and_integrate_moves`/`field_life_*`）**留在 `world.rs`**——它们测的是写 API、`push_*`、
或跨相位的 step 级集成。**核对：总数仍是 103。**

- [ ] **Step 5: 验收（最终）**

Run:
```powershell
cargo test --workspace 2>&1 | Select-String "test result: ok"
cargo run -p stg-harness -- golden --out $env:TEMP\after4.txt
fc.exe $env:TEMP\baseline.txt $env:TEMP\after4.txt
cargo run -p stg-harness -- verify-tables
```
Expected: **103 passed**；`fc` 报无差异（**与最初基线**，不是与 after3）；verify-tables 一致。

- [ ] **Step 6: 报告最终行数**

Run: `wc -l crates/stg-core/src/world.rs crates/stg-core/src/world/*.rs`
在报告里给出拆分后各文件行数——目标是 `world.rs` 从 1593 降到 ~400 以内，没有单个文件超过 ~400。

- [ ] **Step 7: fmt + clippy** — 零警告。

- [ ] **Step 8: Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/world/integrate.rs crates/stg-core/src/world/cleanup.rs
git commit -m "refactor(world): 抽出 world/integrate.rs（相位5）+ world/cleanup.rs（相位9）

world.rs 1593 → 模块结构镜像相位骨架。纯搬家：金向量校验和逐字节不变、103 测试不变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 9: 推分支 + CI 三平台绿 + 用户确认后合并**

推 `refactor/m0-9-world-module-split`（需开 PR 才触发 CI —— 工作流只在 `push: main` / `pull_request` 上跑）。
确认 `lint`/`vector`×3/`determinism-gate` 全绿。**用户点头后** ff 合并 main 并删分支。

---

## Self-Review

**1. 覆盖：** 设计的 6 个文件全部有对应任务（world.rs 保留项 T1-T4 逐步瘦身；collide T1；settle T2；player T3；integrate+cleanup T4）✓ 场界常量提升 T3 ✓ test_support T1（提前，因 T1 的测试就需要）✓ 命名决策（`world/player.rs`）写进 T3 文件头 ✓ 不碰 A8 角色分发 —— T3 明写 ✓

**2. Placeholder scan：** 无 TBD/TODO。每步有精确命令与期望输出。文件头注释给了实际文本；方法体因是"原样剪切"故不重复粘贴（重复 800 行代码进计划只会诱发手抄错误——**剪切**才是这里的正确动作，且校验和基线是它的验收）。

**3. 一致性：**
- `mod` 声明分刀追加（T1 只加 `mod collide;`）——T1 Step 3 已明确标注，避免声明不存在的文件导致编译失败 ✓
- `test_support` 在 T1 建立，T2/T3/T4 复用 ✓
- 场界常量在 T3 提升，T4 的 cleanup.rs 消费 ✓（T3 先于 T4，顺序正确）
- 每刀验收都对**最初基线**比对，不是对上一刀 ✓

**4. 已知取舍：**
- `step.rs`（275 行）不动——它不胖，且相位顺序是宪法，动它风险不成比例。
- `world.rs` 保留的 `run_transforms`(相4 stub)/`begin`/`advance` 各仅数行，不值得单开文件；变换系统那刀落地时 `run_transforms` 自然会长大并独立。
- M0-8 复审留下的 Minor（`ROW_SHOT_ENEMY`/`ROW_FIELD_ENEMY` 两 arm 的 2 行门禁重复）**不在本刀修**——那是行为相邻的改动，会污染"纯搬家"的验收。留作 follow-up。
