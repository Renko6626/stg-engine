# ECL 资产管线（C11 / Spec 2）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 stg-core 从**规范字节文件**反序列化出 owned `WorldTables` 运行（不再认编译进去的 `static`），并用 `content_hash` 机制性焊死"编译 `.ecl` 绑定的表 == 运行时加载的表"。

**Architecture:** owned 化 `WorldTables`（`&'static` 切片→`Box`）；harness 把 `build_tables_v0()` 的输出烘成规范 i32/u16 小端 `tables_v0.bin`（提交、CI 逐位 verify）；stg-core `from_bytes(include_bytes!)` 加载并计算 vendored FNV-1a64 `content_hash`；`compile_for_table` 把表 hash 盖进 `EclImage`；`World::new_with_tables` 记 `tables_hash`；`start_main` 一次比对（`0=未绑定`逃逸）。三层常量（①引擎结构/②表符号/③数据表）+ join 防迷路。

**Tech Stack:** Rust 2024 (toolchain 1.92.0)；stg-core（断层线以下，无 float）；stg-ecl-compiler；stg-harness（断层线以上，bake/verify）；vendored `checksum::Fnv1a64`；`std::sync::LazyLock`。

## Global Constraints

> 每个 task 的要求都隐含包含本节。值逐字取自 spec / CLAUDE.md。

- **I1 定点**：stg-core 内**不得出现 `f32`/`f64`**。`from_bytes` 只读整数（i32/u16/u8/u64），绝不解析浮点。float 只活在 harness（本 plan v1 连 harness 都不用 float——Fx 直抄现值）。
- **I7 布局**：表**不进 `World`**，每帧以 `&WorldTables` 引用穿线；`World` 内无引用/堆容器。唯一例外新增字段 `World.tables_hash: u64`（标量，正常入校验和）。
- **校验和契约**：vendored `checksum::Fnv1a64`（算法字节冻结，绝不换外部依赖）；小端字节序；`content_hash` 复用它。
- **无 committed 金向量基线**：determinism-gate 只三平台互比（CLAUDE.md）。纯重构任务（组 A/B）须**金向量逐位不变**（本地回归证未改值）；组 C 起新增 `World` 字段会合法地移动（未提交、不破基线）的校验和流。
- **确定性计数**：`ITEM_TYPE_COUNT == 5`（`items.rs:22`）；v1 `characters` 定长 1；`appearances` v0 恰 4 行（`APPEARANCE_SMALL/MEDIUM/LARGE/STAR = 0/1/2/3`）。
- **烘焙字节纪律**：表字节生成一次→提交→`include_bytes!` 消费→CI 重烘逐位比对（同数学表 `sin_quarter.bin`）。
- **commit 结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- **README.md 保持未暂存**（有无关的既有改动，勿 `git add`）。
- **金向量命令**（各任务验证用）：`cargo run -p stg-harness -- golden --out /tmp/c.txt` 输出逐帧校验和；比对用 `diff`。

## 文件结构（改哪些、各司其职）

- `crates/stg-core/src/tables.rs`（**改**）：owned `WorldTables` 结构；`build_tables_v0()`（v0 内容单一真相源，Fx 直写）；`to_bytes`/`from_bytes`/`TableLoadError`；`TABLES_V0: LazyLock`；扩 `validate()`（join 腿）。
- `crates/stg-core/src/tables/tables_v0.bin`（**建**）：烘焙产出、提交进仓库。
- `crates/stg-core/src/consts.rs`（**改**）：`engine_consts!` 分 `ENGINE_STRUCTURAL`(①)/`TABLE_SYMBOLS`(②)，`ENGINE_CONSTS = ①⧺②`。
- `crates/stg-core/src/step.rs`（**改**）：`World.tables_hash` 字段；`new_with_tables(seed, &WorldTables)`；`new` 委托。
- `crates/stg-core/src/ecl/binding.rs`（**改**）：`start_main`/`start_main_with_owner` 加 coherence 守卫；`TaskStartError::TableImageMismatch`。
- `crates/stg-core/src/{world/*,ecl/*,player.rs,…}`（**改**，机械）：~175 处 `&TABLES_V0`→`&*TABLES_V0` + `Copy`-move→`.clone()`。
- `crates/stg-ecl-compiler/src/lang/{mod.rs,codegen.rs}` + `src/lib.rs`（**改**）：`content_hash` 透传 `codegen::generate`→`ImageBuilder::build`；新增 `compile_for_table`。
- `crates/stg-harness/src/tables.rs`（**改**）：`tables_v0.bin` 进 bake/verify registry（烘 `stg_core::tables::build_tables_v0().to_bytes()`）。
- `crates/stg-harness/src/main.rs`（**改**）：golden 端到端从磁盘 `from_bytes` 载表跑一遍自证。
- 收口文档：`PROGRESS.md` / `docs/follow-ups.md` / `docs/ecl-lang.md`。

---

## 组 A — owned 化（金向量逐位不变的纯重构）

### Task A1: `WorldTables` owned 化 + `build_tables_v0()` + `LazyLock`

**Files:**
- Modify: `crates/stg-core/src/tables.rs`（结构 + 内容 + 测试）
- Modify（机械，编译器驱动）：`crates/stg-core/src/step.rs:50`、`crates/stg-core/src/world/*.rs`、`crates/stg-core/src/ecl/*.rs`、`crates/stg-core/src/player.rs`、`crates/stg-ecl-compiler/src/lib.rs`、`crates/stg-ecl-compiler/src/lang/codegen.rs`、`crates/stg-harness/src/main.rs`（全部 `&TABLES_V0` 站点）

**Interfaces:**
- Produces：
  - `pub struct WorldTables { pub content_hash: u64, pub characters: [CharacterCfg; 1], pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT], pub drop_tables: Box<[Box<[(u8, u8)]>]>, pub item_gravity: Fx, pub appearances: Box<[AppearanceCfg]> }`
  - `pub fn build_tables_v0() -> WorldTables`（v0 内容单一真相源；`content_hash` 暂置 0，组 B 由 `from_bytes` 计算）
  - `pub static TABLES_V0: std::sync::LazyLock<WorldTables>`（本任务 = `LazyLock::new(build_tables_v0)`；组 B 换 `from_bytes`）
  - `ShotTypeCfg`/`CharacterCfg` 去 `Copy`、留 `Clone`；`AppearanceCfg`/`ItemTypeCfg`/`Shooter` 仍 `Copy`
- Consumes：`stg_core::consts::{APPEARANCE_SMALL, APPEARANCE_MEDIUM, APPEARANCE_LARGE, APPEARANCE_STAR}`（既有）；`crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT}`（既有）

- [ ] **Step 1: 抓改动前金向量作回归 oracle**

Run:
```bash
cargo run -q -p stg-harness -- golden --out /tmp/golden_before.txt && wc -l /tmp/golden_before.txt
```
Expected: 成功输出逐帧校验和文件（若 golden 子命令因后续步骤中途不编译，本步须在动手前先跑）。留作 Step 8 比对基线。

- [ ] **Step 2: 重定义结构 + `build_tables_v0()` + `LazyLock`（tables.rs 顶部）**

把 `tables.rs` 从模块文档到 `TABLES_V0`（含所有 `static TIER_*`/`const *_V0`）**整段替换**为下方 owned 形态。关键变化：`&'static` 切片→`Box<[..]>`；`sets`/`option_pos` 每槽独立 `Box`（不再 `ptr::eq` 共享）；`ShotTypeCfg`/`CharacterCfg` 去 `Copy`；`static TABLES_V0`→`LazyLock`；`appearances` 按 `②` const 下标赋值（防 FM2）。

```rust
//! `WorldTables` —— 世界静态数据层（A3）。**owned 形态**（C11）：内部切片 `Box` 拥有，可由
//! `from_bytes` 在运行时反序列化构造，不再是编译期 `&'static`。传递形态不变：**不进 `World`**
//! （I7），`step`/相位按帧以 `&WorldTables` 引用消费。`content_hash`：组 A 恒 0，组 B 由
//! `from_bytes` 计算并校验（LIVE），coherence 守卫读它。

use std::sync::LazyLock;

// `pub use` 保留原有再导出（消费者可能引 `crate::tables::APPEARANCE_*`），且在本模块内可用。
pub use crate::consts::{APPEARANCE_LARGE, APPEARANCE_MEDIUM, APPEARANCE_SMALL, APPEARANCE_STAR};
use crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT};
use crate::math::{Angle, Fx};
use crate::world::MAX_ENTITY_RADIUS;

/// 全局静态数据层（A3；owned）。见模块文档「传递形态」。
pub struct WorldTables {
    /// 内容哈希：组 B 起 LIVE（`from_bytes` 算 body 的 FNV-1a64 并自校）；组 A 恒 0。
    pub content_hash: u64,
    /// v0 一个角色；定长数组（引擎固定计数，多角色=未来）。
    pub characters: [CharacterCfg; 1],
    pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT],
    pub drop_tables: Box<[Box<[(u8, u8)]>]>,
    pub item_gravity: Fx,
    /// 弹外观表（索引 = appearance id）。
    pub appearances: Box<[AppearanceCfg]>,
}

#[derive(Clone, Copy, Debug)]
pub struct AppearanceCfg {
    pub radius: Fx,
    pub sprite: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ItemTypeCfg {
    pub score: u32,
    pub eject_speed: Fx,
    pub terminal_vy: Fx,
    pub magnet_speed: Fx,
    pub pickup_radius: Fx,
    pub attract_radius: Fx,
}

/// 单角色配置。owned 化后含 `ShotTypeCfg`（有 `Box`）→ **去 `Copy`、留 `Clone`**。
#[derive(Clone, Debug)]
pub struct CharacterCfg {
    pub high_speed: Fx,
    pub low_speed: Fx,
    pub inv_sqrt2: Fx,
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub shot: ShotTypeCfg,
}

/// shottype 表：5 档 × 2 焦点 = 10 槽。owned 化后每槽独立 `Box`（**去 `Copy`、留 `Clone`**）。
#[derive(Clone, Debug)]
pub struct ShotTypeCfg {
    /// `[tier 0..=4][focus 0/1]`；owned 后两焦点槽各持独立分配、内容相等（不再 `ptr::eq` 同一）。
    pub sets: [[Box<[Shooter]>; 2]; 5],
    /// 每档子机偏移（`option` 号 1..=len 查此表；v0 前四档空）。
    pub option_pos: [Box<[(Fx, Fx)]>; 5],
}

#[derive(Clone, Copy, Debug)]
pub struct Shooter {
    pub interval: u16,
    pub delay: u16,
    pub dx: Fx,
    pub dy: Fx,
    pub angle: Angle,
    pub speed: Fx,
    pub damage: u16,
    pub radius: Fx,
    pub sprite: u16,
    pub option: u8,
    pub flags: u8,
}

// ── v0 内容基元（保持现值逐字节不变）─────────────────────────────────────────

const BASE_SHOOTER: Shooter = Shooter {
    interval: 4,
    delay: 0,
    dx: Fx::ZERO,
    dy: Fx::ZERO,
    angle: Angle(49152),
    speed: Fx::from_int(12),
    damage: 1,
    radius: Fx::from_int(4),
    sprite: 0,
    option: 0,
    flags: 0,
};

const ITEM_GRAVITY_V0: Fx = Fx::from_raw(9_830);

const STD_ITEM: ItemTypeCfg = ItemTypeCfg {
    score: 0,
    eject_speed: Fx::from_int(3),
    terminal_vy: Fx::from_raw(144_179),
    magnet_speed: Fx::from_int(8),
    pickup_radius: Fx::from_int(16),
    attract_radius: Fx::from_int(40),
};

const ITEM_CFG_V0: [ItemTypeCfg; ITEM_TYPE_COUNT] = [
    ItemTypeCfg { score: 10, ..STD_ITEM },  // POWER
    ItemTypeCfg { score: 100, ..STD_ITEM }, // POINT
    ItemTypeCfg { score: 50, ..STD_ITEM },  // LIFE_PIECE
    ItemTypeCfg { score: 50, ..STD_ITEM },  // BOMB_PIECE
    ItemTypeCfg { score: 30, ..STD_ITEM },  // STAR
];

const CHAR0_HIGH_SPEED: Fx = Fx::from_raw(294_912);
const CHAR0_LOW_SPEED: Fx = Fx::from_raw(131_072);
const CHAR0_INV_SQRT2: Fx = Fx::from_raw(46_341);
const CHAR0_HIT_RADIUS: Fx = Fx::from_raw(163_840);
const CHAR0_GRAZE_RADIUS: Fx = Fx::from_int(16);

/// tier 4 子机出生偏移（唯一非空档）。
const TIER4_OPT: (Fx, Fx) = (Fx::from_int(-20), Fx::from_int(8));

/// v0 全内容 owned 构造（**单一真相源**）：harness 烘焙与运行期 `from_bytes` round-trip 皆以此为准。
/// `content_hash` 置 0（组 B 起由 `from_bytes` 计算填真值）。
pub fn build_tables_v0() -> WorldTables {
    // 各档发射器列表（owned；每次调用新分配，两焦点槽各自持有）。
    let tier_1way = || -> Box<[Shooter]> { Box::new([BASE_SHOOTER]) };
    let tier_2way = || -> Box<[Shooter]> {
        Box::new([
            Shooter { dx: Fx::from_int(-8), ..BASE_SHOOTER },
            Shooter { dx: Fx::from_int(8), ..BASE_SHOOTER },
        ])
    };
    let tier_4way = || -> Box<[Shooter]> {
        Box::new([
            Shooter { dx: Fx::from_int(-12), ..BASE_SHOOTER },
            BASE_SHOOTER,
            Shooter { dx: Fx::from_int(12), ..BASE_SHOOTER },
            Shooter { option: 1, ..BASE_SHOOTER },
        ])
    };
    let empty_opt = || -> Box<[(Fx, Fx)]> { Box::new([]) };

    let shot = ShotTypeCfg {
        sets: [
            [tier_1way(), tier_1way()],
            [tier_1way(), tier_1way()],
            [tier_2way(), tier_2way()],
            [tier_2way(), tier_2way()],
            [tier_4way(), tier_4way()],
        ],
        option_pos: [
            empty_opt(),
            empty_opt(),
            empty_opt(),
            empty_opt(),
            Box::new([TIER4_OPT]),
        ],
    };

    // appearances 按 `②` const 下标赋值（防 FM2：const 即下标，结构上无法错序）。
    let mut appearances = [AppearanceCfg { radius: Fx::ZERO, sprite: 0 }; 4];
    appearances[APPEARANCE_SMALL as usize] = AppearanceCfg { radius: Fx::from_int(3), sprite: 0 };
    appearances[APPEARANCE_MEDIUM as usize] = AppearanceCfg { radius: Fx::from_int(4), sprite: 1 };
    appearances[APPEARANCE_LARGE as usize] = AppearanceCfg { radius: Fx::from_int(6), sprite: 2 };
    appearances[APPEARANCE_STAR as usize] = AppearanceCfg { radius: Fx::from_int(8), sprite: 3 };

    let drop_tables: Box<[Box<[(u8, u8)]>]> = Box::new([
        Box::new([]) as Box<[(u8, u8)]>,
        Box::new([(ITEM_POWER, 2), (ITEM_POINT, 1)]) as Box<[(u8, u8)]>,
    ]);

    WorldTables {
        content_hash: 0,
        characters: [CharacterCfg {
            high_speed: CHAR0_HIGH_SPEED,
            low_speed: CHAR0_LOW_SPEED,
            inv_sqrt2: CHAR0_INV_SQRT2,
            hit_radius: CHAR0_HIT_RADIUS,
            graze_radius: CHAR0_GRAZE_RADIUS,
            shot,
        }],
        item_cfg: ITEM_CFG_V0,
        drop_tables,
        item_gravity: ITEM_GRAVITY_V0,
        appearances: Box::new(appearances),
    }
}

/// 内建默认表。组 A：Rust 直构（owned）；组 B 换 `from_bytes(include_bytes!)` 走字节路径。
pub static TABLES_V0: LazyLock<WorldTables> = LazyLock::new(build_tables_v0);
```

`radius_in_range` 不变。`validate()` **改一处**：原 `for shooter in c.shot.sets[tier][focus]`（`sets` 曾是 `&'static [Shooter]` 可直接迭代）现是 `Box<[Shooter]>`，不能移动出——改为 `for shooter in c.shot.sets[tier][focus].iter()`。其余（`appearances.iter()`/`drop_tables.iter().flat_map(|t| t.iter())`）经 `Deref` 透明不变。组 D 再扩 join 腿。

- [ ] **Step 3: 迁移 tables.rs 自有测试（`ptr::eq`→值相等 + 构造字面量→builder）**

`tests` 模块里三类点要改：① 所有 `TABLES_V0.xxx` 现经 `LazyLock` `Deref` 仍可读（`TABLES_V0.appearances` 等无需改）；② `tables_v0_shape` 的 `std::ptr::eq(a, b)` 断言（两焦点槽同一列表）改为**值相等** `a == b`（`Shooter` 派生 `PartialEq`——若未派生，本步给 `Shooter` 加 `#[derive(PartialEq, Eq)]`）；③ `validate_rejects_bad` 里手构 `WorldTables { .. }` 字面量的 `characters: [character]`（`character` 来自 `TABLES_V0.characters[0]` 的 `Copy` 移动）改为 `.clone()`，且 `drop_tables`/`appearances` 字段用 `build_tables_v0()` 的对应字段克隆或重建。

把 `tables_v0_shape` 里：
```rust
assert!(std::ptr::eq(a, b), "tier {tier} 两焦点槽必须指同一列表（v0 简化）");
```
换成：
```rust
assert_eq!(a, b, "tier {tier} 两焦点槽内容相等（owned 后各自独立分配）");
```
并给 `Shooter` 加 `#[derive(PartialEq, Eq)]`（若尚无）。`bad_with_shot` 辅助改为基于 `build_tables_v0()` 克隆：
```rust
fn bad_with_shot(shot: ShotTypeCfg) -> WorldTables {
    let mut t = build_tables_v0();
    t.characters[0].shot = shot;
    t
}
```
`validate_rejects_bad_appearance_radius` 同理用 `build_tables_v0()` 起手改 `appearances`：
```rust
let mut bad = build_tables_v0();
bad.appearances = Box::new([AppearanceCfg { radius: Fx::from_int(2000), sprite: 0 }]);
assert!(!bad.validate(), "appearance 半径超上限必须被 validate 拒绝");
```
`CHARACTER0_SHOT`/`static BAD_*` 那些 `static [Shooter;N]` 辅助改为局部 `let bad: Box<[Shooter]> = Box::new([Shooter { interval: 0, ..BASE_SHOOTER }]);` 再 `shot.sets[0] = [bad.clone(), bad];`。

- [ ] **Step 4: 编译 stg-core，编译器驱动修内部站点**

Run:
```bash
cargo build -p stg-core 2>&1 | grep -E "error\[|--> " | head -60
```
Expected: 报错集中在两类，逐个修：
1. **`&TABLES_V0` 当整表引用**（`E0308` 期望 `&WorldTables` 得 `&LazyLock<WorldTables>`）→ 改 `&*TABLES_V0` / `&*crate::tables::TABLES_V0`。**注意**：`&crate::tables::TABLES_V0.characters[0]` 这种**字段访问**经 `Deref` 自动生效、**不改**（加 `*` 反而错）。典型点：`step.rs:50` 的 `PlayerState::spawn(0, &crate::tables::TABLES_V0.characters[0])` 无需改（字段访问）。
2. **`Copy` 移动失效**（`E0507` cannot move out of，如 `let c = TABLES_V0.characters[0]`）→ 加 `.clone()`。

反复 `cargo build -p stg-core` 直至 0 error。

- [ ] **Step 5: 编译整 workspace，修 harness/编译器站点**

Run:
```bash
cargo build --workspace 2>&1 | grep -E "error\[|--> " | head -40
```
Expected: `crates/stg-ecl-compiler/src/lib.rs`、`src/lang/codegen.rs`、`crates/stg-harness/src/main.rs` 里的 `&stg_core::tables::TABLES_V0`（整表引用）→ `&*stg_core::tables::TABLES_V0`。字段访问点不改。反复直至 0 error。

- [ ] **Step 6: 全测试绿**

Run:
```bash
cargo test --workspace 2>&1 | tail -20
```
Expected: 全 pass（数与改动前一致，无新增/丢失用例——除 `Shooter` 加 `PartialEq` 不影响计数）。

- [ ] **Step 7: fmt + clippy**

Run:
```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: 无 warning。

- [ ] **Step 8: 金向量逐位不变（回归闸门）**

Run:
```bash
cargo run -q -p stg-harness -- golden --out /tmp/golden_after.txt && diff -u /tmp/golden_before.txt /tmp/golden_after.txt && echo "IDENTICAL"
```
Expected: `IDENTICAL`（无 diff）。owned 化是纯重构，值一字不改——这是本任务最硬的正确性证明。

- [ ] **Step 9: Commit**

```bash
git add crates/stg-core/src/tables.rs crates/stg-core/src/step.rs crates/stg-core/src/world crates/stg-core/src/ecl crates/stg-core/src/player.rs crates/stg-ecl-compiler/src crates/stg-harness/src/main.rs
git commit -m "refactor(tables): own WorldTables internals, TABLES_V0 as LazyLock

&'static slices -> Box; ShotTypeCfg/CharacterCfg drop Copy; TABLES_V0
built via build_tables_v0() behind LazyLock. Golden byte-identical.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 组 B — 规范字节 + 真 content_hash

### Task B1: `to_bytes` / `from_bytes` / `TableLoadError`

**Files:**
- Modify: `crates/stg-core/src/tables.rs`（加 `TableLoadError`、私有 `Reader`、`to_bytes`/`from_bytes`/`write_shooter`/`read_shooter`；给 `WorldTables` 及子结构加 `PartialEq, Eq`）
- Test: `crates/stg-core/src/tables.rs`（`#[cfg(test)] mod tests` 追加）

**Interfaces:**
- Consumes：`build_tables_v0()`（A1）；`crate::checksum::Fnv1a64`；`ITEM_TYPE_COUNT`
- Produces：
  - `pub fn WorldTables::to_bytes(&self) -> Vec<u8>`（规范 i32/u16 小端，无 float）
  - `pub fn WorldTables::from_bytes(buf: &[u8]) -> Result<WorldTables, TableLoadError>`（只读整数；算 body FNV 自校；末尾 `validate()`）
  - `pub enum TableLoadError { Truncated, BadMagic, UnsupportedVersion(u16), HashMismatch, ArityMismatch { field: &'static str, expected: usize, actual: usize }, ValidateFailed }`

- [ ] **Step 1: 给结构加 `PartialEq, Eq`（round-trip 断言前提）**

给 `WorldTables`、`CharacterCfg`、`ShotTypeCfg`、`AppearanceCfg`、`ItemTypeCfg` 的 `#[derive(..)]` 补 `PartialEq, Eq`（`Fx`/`Angle` 均 `Eq`；`Box<[T]>` 在 `T: PartialEq` 时 `PartialEq`）。`WorldTables` 本无 derive，新加 `#[derive(PartialEq, Eq)]`。**`Shooter` 已在 A1 Step 3 加过 `PartialEq, Eq`——勿重复 derive。**

- [ ] **Step 2: Write failing round-trip test**

在 tests 模块加：
```rust
#[test]
fn to_from_bytes_round_trip_preserves_all_fields() {
    let mut t = build_tables_v0();
    let bytes = t.to_bytes();
    let back = WorldTables::from_bytes(&bytes).expect("round-trip must load");
    assert_ne!(back.content_hash, 0, "from_bytes 计算真 content_hash");
    t.content_hash = back.content_hash; // 对齐 from_bytes 填的唯一字段
    assert_eq!(t, back, "round-trip 逐字段一致");
}

#[test]
fn to_bytes_is_deterministic() {
    assert_eq!(build_tables_v0().to_bytes(), build_tables_v0().to_bytes());
}
```

- [ ] **Step 3: Run — verify it fails to compile (functions absent)**

Run: `cargo test -p stg-core tables::tests::to_from 2>&1 | tail -5`
Expected: FAIL —— `no function or associated item named to_bytes`。

- [ ] **Step 4: 实现 `TableLoadError` + `Reader` + `to_bytes`/`from_bytes`**

在 tables.rs（`validate` 附近、tests 之前）加：

```rust
/// 表加载错误（构造前资产环节；返 Result 不 panic，不触模拟确定性）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableLoadError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u16),
    HashMismatch,
    ArityMismatch { field: &'static str, expected: usize, actual: usize },
    ValidateFailed,
}

/// 规范字节读取游标（小端；越界→Truncated）。
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], TableLoadError> {
        let end = self.p.checked_add(n).ok_or(TableLoadError::Truncated)?;
        let s = self.b.get(self.p..end).ok_or(TableLoadError::Truncated)?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, TableLoadError> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, TableLoadError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, TableLoadError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, TableLoadError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn fx(&mut self) -> Result<Fx, TableLoadError> { Ok(Fx::from_raw(self.i32()?)) }
    fn angle(&mut self) -> Result<Angle, TableLoadError> { Ok(Angle(self.u16()?)) }
}

fn write_shooter(out: &mut Vec<u8>, s: &Shooter) {
    out.extend_from_slice(&s.interval.to_le_bytes());
    out.extend_from_slice(&s.delay.to_le_bytes());
    out.extend_from_slice(&s.dx.raw().to_le_bytes());
    out.extend_from_slice(&s.dy.raw().to_le_bytes());
    out.extend_from_slice(&s.angle.raw().to_le_bytes());
    out.extend_from_slice(&s.speed.raw().to_le_bytes());
    out.extend_from_slice(&s.damage.to_le_bytes());
    out.extend_from_slice(&s.radius.raw().to_le_bytes());
    out.extend_from_slice(&s.sprite.to_le_bytes());
    out.push(s.option);
    out.push(s.flags);
}

fn read_shooter(r: &mut Reader) -> Result<Shooter, TableLoadError> {
    Ok(Shooter {
        interval: r.u16()?,
        delay: r.u16()?,
        dx: r.fx()?,
        dy: r.fx()?,
        angle: r.angle()?,
        speed: r.fx()?,
        damage: r.u16()?,
        radius: r.fx()?,
        sprite: r.u16()?,
        option: r.u8()?,
        flags: r.u8()?,
    })
}

/// 头 16B：magic(4) + version(2) + reserved(2) + content_hash(8)。body = 其后全部字节。
const TABLE_MAGIC: &[u8; 4] = b"STGT";
const TABLE_VERSION: u16 = 1;
const TABLE_HEADER: usize = 16;

impl WorldTables {
    /// 序列化为规范字节（i32/u16 小端，无 float）。`content_hash` = FNV-1a64(body) 回填。
    pub fn to_bytes(&self) -> Vec<u8> {
        use crate::checksum::Fnv1a64;
        let mut out = Vec::new();
        out.extend_from_slice(TABLE_MAGIC);
        out.extend_from_slice(&TABLE_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&0u64.to_le_bytes()); // content_hash 占位（偏移 8..16）

        out.extend_from_slice(&self.item_gravity.raw().to_le_bytes());
        out.extend_from_slice(&(self.appearances.len() as u32).to_le_bytes());
        for a in self.appearances.iter() {
            out.extend_from_slice(&a.radius.raw().to_le_bytes());
            out.extend_from_slice(&a.sprite.to_le_bytes());
        }
        out.extend_from_slice(&(self.item_cfg.len() as u32).to_le_bytes());
        for it in self.item_cfg.iter() {
            out.extend_from_slice(&it.score.to_le_bytes());
            out.extend_from_slice(&it.eject_speed.raw().to_le_bytes());
            out.extend_from_slice(&it.terminal_vy.raw().to_le_bytes());
            out.extend_from_slice(&it.magnet_speed.raw().to_le_bytes());
            out.extend_from_slice(&it.pickup_radius.raw().to_le_bytes());
            out.extend_from_slice(&it.attract_radius.raw().to_le_bytes());
        }
        out.extend_from_slice(&(self.drop_tables.len() as u32).to_le_bytes());
        for tbl in self.drop_tables.iter() {
            out.extend_from_slice(&(tbl.len() as u32).to_le_bytes());
            for &(ty, qty) in tbl.iter() {
                out.push(ty);
                out.push(qty);
            }
        }
        out.extend_from_slice(&(self.characters.len() as u32).to_le_bytes());
        for c in self.characters.iter() {
            out.extend_from_slice(&c.high_speed.raw().to_le_bytes());
            out.extend_from_slice(&c.low_speed.raw().to_le_bytes());
            out.extend_from_slice(&c.inv_sqrt2.raw().to_le_bytes());
            out.extend_from_slice(&c.hit_radius.raw().to_le_bytes());
            out.extend_from_slice(&c.graze_radius.raw().to_le_bytes());
            for tier in 0..5 {
                for focus in 0..2 {
                    let list = &c.shot.sets[tier][focus];
                    out.extend_from_slice(&(list.len() as u32).to_le_bytes());
                    for s in list.iter() {
                        write_shooter(&mut out, s);
                    }
                }
            }
            for tier in 0..5 {
                let op = &c.shot.option_pos[tier];
                out.extend_from_slice(&(op.len() as u32).to_le_bytes());
                for &(x, y) in op.iter() {
                    out.extend_from_slice(&x.raw().to_le_bytes());
                    out.extend_from_slice(&y.raw().to_le_bytes());
                }
            }
        }

        let mut h = Fnv1a64::new();
        h.write_bytes(&out[TABLE_HEADER..]);
        out[8..TABLE_HEADER].copy_from_slice(&h.finish().to_le_bytes());
        out
    }

    /// 从规范字节反序列化（只读整数，守 I1）。校验 magic/version、自校 body FNV、arity、`validate`。
    pub fn from_bytes(buf: &[u8]) -> Result<WorldTables, TableLoadError> {
        use crate::checksum::Fnv1a64;
        if buf.len() < TABLE_HEADER {
            return Err(TableLoadError::Truncated);
        }
        if &buf[0..4] != TABLE_MAGIC {
            return Err(TableLoadError::BadMagic);
        }
        let version = u16::from_le_bytes(buf[4..6].try_into().unwrap());
        if version != TABLE_VERSION {
            return Err(TableLoadError::UnsupportedVersion(version));
        }
        let stored = u64::from_le_bytes(buf[8..TABLE_HEADER].try_into().unwrap());
        let mut h = Fnv1a64::new();
        h.write_bytes(&buf[TABLE_HEADER..]);
        if h.finish() != stored {
            return Err(TableLoadError::HashMismatch);
        }

        let mut r = Reader { b: buf, p: TABLE_HEADER };
        let item_gravity = r.fx()?;

        let na = r.u32()? as usize;
        let mut appearances = Vec::with_capacity(na);
        for _ in 0..na {
            appearances.push(AppearanceCfg { radius: r.fx()?, sprite: r.u16()? });
        }

        let ni = r.u32()? as usize;
        if ni != ITEM_TYPE_COUNT {
            return Err(TableLoadError::ArityMismatch { field: "item_cfg", expected: ITEM_TYPE_COUNT, actual: ni });
        }
        let mut item_vec = Vec::with_capacity(ni);
        for _ in 0..ni {
            item_vec.push(ItemTypeCfg {
                score: r.u32()?,
                eject_speed: r.fx()?,
                terminal_vy: r.fx()?,
                magnet_speed: r.fx()?,
                pickup_radius: r.fx()?,
                attract_radius: r.fx()?,
            });
        }
        let item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT] =
            item_vec.try_into().expect("count checked == ITEM_TYPE_COUNT");

        let nd = r.u32()? as usize;
        let mut drops: Vec<Box<[(u8, u8)]>> = Vec::with_capacity(nd);
        for _ in 0..nd {
            let inner = r.u32()? as usize;
            let mut row = Vec::with_capacity(inner);
            for _ in 0..inner {
                row.push((r.u8()?, r.u8()?));
            }
            drops.push(row.into_boxed_slice());
        }

        let nc = r.u32()? as usize;
        if nc != 1 {
            return Err(TableLoadError::ArityMismatch { field: "characters", expected: 1, actual: nc });
        }
        let high_speed = r.fx()?;
        let low_speed = r.fx()?;
        let inv_sqrt2 = r.fx()?;
        let hit_radius = r.fx()?;
        let graze_radius = r.fx()?;
        let mut sets: [[Box<[Shooter]>; 2]; 5] =
            std::array::from_fn(|_| std::array::from_fn(|_| Box::default()));
        for tier in 0..5 {
            for focus in 0..2 {
                let n = r.u32()? as usize;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(read_shooter(&mut r)?);
                }
                sets[tier][focus] = v.into_boxed_slice();
            }
        }
        let mut option_pos: [Box<[(Fx, Fx)]>; 5] = std::array::from_fn(|_| Box::default());
        for tier in 0..5 {
            let m = r.u32()? as usize;
            let mut v = Vec::with_capacity(m);
            for _ in 0..m {
                v.push((r.fx()?, r.fx()?));
            }
            option_pos[tier] = v.into_boxed_slice();
        }

        let t = WorldTables {
            content_hash: stored,
            characters: [CharacterCfg {
                high_speed,
                low_speed,
                inv_sqrt2,
                hit_radius,
                graze_radius,
                shot: ShotTypeCfg { sets, option_pos },
            }],
            item_cfg,
            drop_tables: drops.into_boxed_slice(),
            item_gravity,
            appearances: appearances.into_boxed_slice(),
        };
        if !t.validate() {
            return Err(TableLoadError::ValidateFailed);
        }
        Ok(t)
    }
}
```

- [ ] **Step 5: Run round-trip tests — verify pass**

Run: `cargo test -p stg-core tables::tests::to_from tables::tests::to_bytes_is 2>&1 | tail -5`
Expected: PASS。

- [ ] **Step 6: Write格式健壮 + hash 敏感测试**

```rust
#[test]
fn content_hash_changes_when_a_value_changes() {
    let h0 = { let b = build_tables_v0().to_bytes(); WorldTables::from_bytes(&b).unwrap().content_hash };
    let mut t = build_tables_v0();
    t.item_gravity = Fx::from_raw(9_831); // 改一个 body 值
    let h1 = { let b = t.to_bytes(); WorldTables::from_bytes(&b).unwrap().content_hash };
    assert_ne!(h0, h1, "改 body 任一值 → content_hash 变");
}

#[test]
fn from_bytes_rejects_bad_magic_version_truncation_and_tamper() {
    let good = build_tables_v0().to_bytes();

    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    assert_eq!(WorldTables::from_bytes(&bad_magic), Err(TableLoadError::BadMagic));

    let mut bad_ver = good.clone();
    bad_ver[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(WorldTables::from_bytes(&bad_ver), Err(TableLoadError::UnsupportedVersion(2)));

    assert_eq!(WorldTables::from_bytes(&good[..8]), Err(TableLoadError::Truncated));

    let mut tampered = good.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF; // 改 body 尾字节但不重算 hash
    assert_eq!(WorldTables::from_bytes(&tampered), Err(TableLoadError::HashMismatch));
}

#[test]
fn from_bytes_rejects_arity_mismatch() {
    // 手工造一份 hash 自洽、但 item_cfg 计数 != ITEM_TYPE_COUNT 的 buffer。
    use crate::checksum::Fnv1a64;
    let mut body = Vec::new();
    body.extend_from_slice(&Fx::ZERO.raw().to_le_bytes()); // item_gravity
    body.extend_from_slice(&0u32.to_le_bytes());           // appearances count 0
    body.extend_from_slice(&3u32.to_le_bytes());           // item_cfg count 3 (!= 5)
    let mut buf = Vec::new();
    buf.extend_from_slice(TABLE_MAGIC);
    buf.extend_from_slice(&TABLE_VERSION.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    let mut h = Fnv1a64::new();
    h.write_bytes(&body);
    buf.extend_from_slice(&h.finish().to_le_bytes());
    buf.extend_from_slice(&body);
    assert_eq!(
        WorldTables::from_bytes(&buf),
        Err(TableLoadError::ArityMismatch { field: "item_cfg", expected: 5, actual: 3 })
    );
}
```

- [ ] **Step 7: Run — all green + fmt/clippy**

Run: `cargo test -p stg-core tables:: 2>&1 | tail -5 && cargo clippy -p stg-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: 全 PASS，无 warning。

- [ ] **Step 8: Commit**

```bash
git add crates/stg-core/src/tables.rs
git commit -m "feat(tables): canonical byte serde + TableLoadError + real content_hash

to_bytes/from_bytes (i32/u16 LE, no float); FNV-1a64 over body, self-checked
on load; arity + validate gates. Round-trip identity + format-robustness tests.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task B2: harness 烘 `tables_v0.bin` + `TABLES_V0` 走字节路径

**Files:**
- Create: `crates/stg-core/src/tables/tables_v0.bin`（烘焙产出，提交）
- Modify: `crates/stg-harness/src/tables.rs`（registry 加 world 表；`bake_all`/`verify_all` 支持任意子目录）
- Modify: `crates/stg-core/src/tables.rs`（`TABLES_V0` 换 `from_bytes(include_bytes!)`）

**Interfaces:**
- Consumes：`stg_core::tables::build_tables_v0()`、`WorldTables::to_bytes`（B1）、`from_bytes`（B1）
- Produces：`crates/stg-core/src/tables/tables_v0.bin`；`TABLES_V0` 现 content_hash 非 0

- [ ] **Step 1: registry 泛化到任意子目录 + 加 world 表生成器**

`crates/stg-harness/src/tables.rs`：把 `registry`/`bake_all`/`verify_all` 从"单一 `TABLES_DIR` + `table_path(name)`"改为携带完整路径。替换这三处：

```rust
/// 世界数据表目录（stg-core/src/tables）。
pub const WORLD_TABLES_DIR: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../stg-core/src/tables");

/// 世界内容表 v0：owned 构造 → 规范字节（单一真相源 = `build_tables_v0`）。
pub fn gen_world_tables_v0() -> Vec<u8> {
    stg_core::tables::build_tables_v0().to_bytes()
}

/// 所有表的 (完整路径, 生成器) 清单——bake 与 verify 共用单一真相源。
fn registry() -> Vec<(PathBuf, TableGen)> {
    let math = PathBuf::from(TABLES_DIR);
    let world = PathBuf::from(WORLD_TABLES_DIR);
    vec![
        (math.join("sin_quarter.bin"), gen_sin_quarter as TableGen),
        (math.join("easing.bin"), gen_easing as TableGen),
        (math.join("atan_cordic.bin"), gen_atan_cordic as TableGen),
        (world.join("tables_v0.bin"), gen_world_tables_v0 as TableGen),
    ]
}

pub fn bake_all() -> std::io::Result<()> {
    for (path, generate) in registry() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let bytes = generate();
        std::fs::write(&path, &bytes)?;
        eprintln!("baked {} ({} bytes)", path.display(), bytes.len());
    }
    Ok(())
}

pub fn verify_all() -> Result<(), String> {
    for (path, generate) in registry() {
        let expected = generate();
        let actual =
            std::fs::read(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        if actual != expected {
            return Err(format!(
                "{} 与 commit 字节不一致（生成 {} vs commit {} 字节）",
                path.display(),
                expected.len(),
                actual.len()
            ));
        }
        eprintln!("verified {} ({} bytes)", path.display(), expected.len());
    }
    Ok(())
}
```
（`table_path` 辅助不再被 registry 用；若无其它调用点，删之。保留 `TABLES_DIR` 常量。）

- [ ] **Step 2: 烘焙并落盘 `tables_v0.bin`**

Run（子命令名若不同，先 `cargo run -p stg-harness -- --help` 确认）：
```bash
cargo run -q -p stg-harness -- bake-tables && ls -l crates/stg-core/src/tables/tables_v0.bin
```
Expected: 打印 `baked .../tables_v0.bin (N bytes)`，文件存在。

- [ ] **Step 3: `TABLES_V0` 换字节加载路径**

`crates/stg-core/src/tables.rs`，把 A1 的
```rust
pub static TABLES_V0: LazyLock<WorldTables> = LazyLock::new(build_tables_v0);
```
换成：
```rust
/// 内建默认表：从提交的规范字节反序列化（**证明 core 跑在加载的字节上**；金向量走此路径）。
pub static TABLES_V0: LazyLock<WorldTables> = LazyLock::new(|| {
    WorldTables::from_bytes(include_bytes!("tables/tables_v0.bin"))
        .expect("baked v0 table must satisfy the runtime contract")
});
```

- [ ] **Step 4: content_hash LIVE 测试**

tests 模块加：
```rust
#[test]
fn builtin_tables_v0_has_live_content_hash() {
    assert_ne!(TABLES_V0.content_hash, 0, "内建表经 from_bytes 载入，hash 应非 0");
    // 与直接烘焙同源的自洽：include 的字节 == build_tables_v0().to_bytes()
    let from_builder = WorldTables::from_bytes(&build_tables_v0().to_bytes()).unwrap();
    assert_eq!(TABLES_V0.content_hash, from_builder.content_hash);
}
```

- [ ] **Step 5: verify-tables + 全测试 + 金向量逐位不变**

Run:
```bash
cargo run -q -p stg-harness -- verify-tables 2>&1 | tail -4
cargo test --workspace 2>&1 | tail -5
cargo run -q -p stg-harness -- golden --out /tmp/golden_B2.txt && diff -u /tmp/golden_before.txt /tmp/golden_B2.txt && echo "IDENTICAL"
```
Expected: verify 全 pass（含 tables_v0.bin）；测试全绿；金向量 `IDENTICAL`（sim 不读 content_hash，值未变）。

- [ ] **Step 6: Commit（含二进制表）**

```bash
git add crates/stg-core/src/tables/tables_v0.bin crates/stg-harness/src/tables.rs crates/stg-core/src/tables.rs
git commit -m "feat(tables): bake tables_v0.bin, load TABLES_V0 via from_bytes

Harness bakes build_tables_v0().to_bytes() to a committed .bin (CI verify,
same discipline as math tables); stg-core loads it via include_bytes!+from_bytes,
so content_hash is now LIVE. Golden byte-identical.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 组 C — compile 绑定表 + coherence 守卫

### Task C1: `content_hash` 透传 codegen + `compile_for_table`

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lib.rs:708`（`ImageBuilder::build` 加 `content_hash` 参）+ `:843`（`content_hash: 0`→变量）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs:603`（`generate` 加参）+ `:680`（`ib.build(content_hash)`）
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（`compile_with_options` 加参；新增 `compile_for_table`；`compile` 委托）
- Test: `crates/stg-ecl-compiler/src/lang/mod.rs`（tests）

**Interfaces:**
- Consumes：`stg_core::tables::{WorldTables, TABLES_V0}`（B2 起 `TABLES_V0.content_hash` LIVE）；`stg_core::consts::ENGINE_CONSTS`
- Produces：
  - `pub fn compile_for_table(src: &str, file: &str, table: &stg_core::tables::WorldTables) -> Result<EclImage, Vec<CompileError>>`
  - `compile(src, file)` 签名不变 = `compile_for_table(src, file, &TABLES_V0)`
  - `compile_with_options(.., content_hash: u64)`（末尾加参）
  - `codegen::generate(.., content_hash: u64)`；`ImageBuilder::build(self, content_hash: u64)`

- [ ] **Step 1: `ImageBuilder::build` 收 `content_hash`**

`lib.rs:708` 签名改：
```rust
pub fn build(mut self, content_hash: u64) -> Result<EclImage, ImageBuildError> {
```
`lib.rs:843` 的 `content_hash: 0,` 改为 `content_hash,`。修 lib.rs 内 `build()` 的既有调用点（tests 里的 `.build()`）→ `.build(0)`（这些 builder 单测不测表绑定，传 0）。

- [ ] **Step 2: `codegen::generate` 透传**

`codegen.rs:603` 签名加参：
```rust
pub fn generate(
    prog: &Program,
    ti: &TypedInfo,
    sm: &SlotMap,
    content_hash: u64,
) -> Result<EclImage, Vec<CompileError>> {
```
`codegen.rs:680` 的 `ib.build()` → `ib.build(content_hash)`。修 codegen.rs tests 里 `generate(..)` 调用点 → 末尾加 `, 0`。

- [ ] **Step 3: `mod.rs` — `compile_with_options` 加参 + `compile_for_table` + `compile` 委托**

`compile_with_options` 加末参 `content_hash: u64`，并把 `codegen::generate(&program, &typed, &slot_map)` 改为 `codegen::generate(&program, &typed, &slot_map, content_hash)`。然后：
```rust
/// 为指定表编译：注入引擎常量（①②）+ 盖 `table.content_hash` 进 `EclImage`（coherence 焊点）。
/// v1 只取 `table.content_hash`（①② 常量仍来自 `consts::ENGINE_CONSTS`）；乙案将来从表读符号。
pub fn compile_for_table(
    src: &str,
    file: &str,
    table: &stg_core::tables::WorldTables,
) -> Result<EclImage, Vec<CompileError>> {
    compile_with_options(
        src,
        file,
        CompileOptions { debug_info: DebugInfo::None },
        stg_core::consts::ENGINE_CONSTS,
        table.content_hash,
    )
    .map(|ce| ce.image)
}

/// 便利包装：绑定内建默认表 `TABLES_V0`。签名不变（既有调用方零改）。
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>> {
    compile_for_table(src, file, &stg_core::tables::TABLES_V0)
}
```
（`&stg_core::tables::TABLES_V0` 是 `&LazyLock<WorldTables>`，经 Deref coercion 当 `&WorldTables` 传入。）
修 `compile_with_options` 的既有调用点（mod.rs 调试侧载 tests 传 `&[]` 的那些）→ 末尾加 `, 0`（unbound）。

- [ ] **Step 4: Write test — 编译镜像携带目标表 hash**

`mod.rs` tests 加（`sub main() {}` 若因空 body 不编译，改用 `sub main() { wait 1; }`）：
```rust
#[test]
fn compiled_image_carries_target_table_content_hash() {
    let img = compile("sub main() {}", "t.ecl").expect("minimal main compiles");
    assert_ne!(img.content_hash(), 0, "绑定 TABLES_V0（LIVE hash）");
    assert_eq!(img.content_hash(), stg_core::tables::TABLES_V0.content_hash);
}
```

- [ ] **Step 5: Run — build + test + clippy**

Run:
```bash
cargo test -p stg-ecl-compiler 2>&1 | tail -6
cargo clippy -p stg-ecl-compiler --all-targets -- -D warnings 2>&1 | tail -3
```
Expected: 全 PASS，无 warning。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-ecl-compiler/src
git commit -m "feat(ecl): compile_for_table stamps table content_hash into EclImage

Thread content_hash through codegen::generate -> ImageBuilder::build.
compile() now binds to TABLES_V0; compile_for_table(&table) for explicit binding.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task C2: `World.tables_hash` + `new_with_tables`

**Files:**
- Modify: `crates/stg-core/src/step.rs`（World 加 `tables_hash` 字段；`new_with_tables`；`new` 委托；`copy_into` 复制该字段）
- Test: `crates/stg-core/src/step.rs`（tests）

**Interfaces:**
- Consumes：`crate::tables::{WorldTables, TABLES_V0}`；`PlayerState::spawn`
- Produces：
  - `World.tables_hash: u64`（`pub(crate)`；入 `#[derive(Checksum)]`——正常参与校验和）
  - `pub fn World::new_with_tables(seed: u64, tables: &crate::tables::WorldTables) -> Box<World>`
  - `new(seed)` = `new_with_tables(seed, &TABLES_V0)`

- [ ] **Step 1: World 加字段 + copy_into 复制**

`step.rs` World 结构（`ecl_main_started` 旁）加：
```rust
    /// 本 World 绑定的表 `content_hash`（`new_with_tables` 记录）。coherence 守卫读它对
    /// `EclImage.content_hash` 一次比对。常量存活期不变，跨机一致，正常入校验和。
    pub(crate) tables_hash: u64,
```
`copy_into`（与 `ecl_main_started` 复制同处）加一行：`dst.tables_hash = self.tables_hash;`
（若 `copy_into` 未显式复制 `ecl_main_started`，检查其如何被复制并照同款加 `tables_hash`——两者都是 World 级标量字段。）

- [ ] **Step 2: `new` 委托 + `new_with_tables`**

把 `new` 整体替换：
```rust
    pub fn new(seed: u64) -> Box<World> {
        Self::new_with_tables(seed, &crate::tables::TABLES_V0)
    }

    /// 用指定表构造：自机取 `tables.characters[0]`，并记录 `tables_hash = tables.content_hash`
    /// 供启动期 coherence 守卫。`new(seed)` 委托此函数绑内建 `TABLES_V0`。
    pub fn new_with_tables(seed: u64, tables: &crate::tables::WorldTables) -> Box<World> {
        let layout = Layout::new::<World>();
        // SAFETY: 同 `new` 原注释——World 全零合法，堆零构造避栈溢出。
        let mut w: Box<World> = unsafe {
            let ptr = alloc_zeroed(layout) as *mut World;
            if ptr.is_null() {
                handle_alloc_error(layout);
            }
            Box::from_raw(ptr)
        };
        w.body.rng = Pcg32::new(seed, RNG_SEQ);
        w.body.players[0] = crate::player::PlayerState::spawn(0, &tables.characters[0]);
        w.tables_hash = tables.content_hash;
        w
    }
```
（`&crate::tables::TABLES_V0` 经 Deref coercion 当 `&WorldTables`。）

- [ ] **Step 3: Write test**

`step.rs` tests 加：
```rust
#[test]
fn new_records_default_table_hash() {
    let w = World::new(0x1234);
    assert_ne!(w.tables_hash, 0);
    assert_eq!(w.tables_hash, crate::tables::TABLES_V0.content_hash);
}
```

- [ ] **Step 4: Run — build + test（金向量流合法移动，不比 before）**

Run:
```bash
cargo test --workspace 2>&1 | tail -6
```
Expected: 全 PASS（含既有快照 round-trip：原/复原 World 同含 `tables_hash`，checksum 一致）。
**注意**：本任务给 World 加了真字段，金向量校验和流会合法地不同于 B2（无 committed 基线，不破任何东西；三平台仍一致）——**不要**再对 `/tmp/golden_before.txt` 比 `IDENTICAL`。

- [ ] **Step 5: fmt/clippy + Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
git add crates/stg-core/src/step.rs
git commit -m "feat(step): World.tables_hash + new_with_tables records bound table hash

new(seed) delegates to new_with_tables(seed, &TABLES_V0). tables_hash is a
World field (checksummed, copied in snapshot); coherence guard reads it.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task C3: 启动期 coherence 守卫 + `TableImageMismatch`

**Files:**
- Modify: `crates/stg-core/src/ecl/binding.rs`（`TaskStartError::TableImageMismatch`；`start_main_with_owner` 加守卫）
- Test: `crates/stg-core/src/ecl/binding.rs`（tests）

**Interfaces:**
- Consumes：`World.tables_hash`（C2）；`EclImage::content_hash()`；`STATUS_BAD_ARGS`
- Produces：`TaskStartError::TableImageMismatch { image: u64, tables: u64 }`；守卫逻辑

- [ ] **Step 1: 加错误变体**

`binding.rs` `TaskStartError` 枚举加：
```rust
    /// The compiled image's content_hash does not match the World's bound table hash.
    TableImageMismatch { image: u64, tables: u64 },
```

- [ ] **Step 2: 在 `start_main_with_owner` 插守卫（main_started 检查之后、root 解析之前）**

`start_main_with_owner` 里，`if self.ecl_main_started != 0 { .. }` 块之后、`let root = image.root()...` 之前插入：
```rust
        // Coherence guard: image compiled for a table whose content_hash must match
        // the table this World was built with. `0` on either side = unbound (empty
        // script / no real table) → skip.  P4-b: caller mismatch → deterministic Err,
        // no panic. Once, at startup — not in the per-frame step path.
        let image_hash = image.content_hash();
        if image_hash != 0 && self.tables_hash != 0 && image_hash != self.tables_hash {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return Err(TaskStartError::TableImageMismatch {
                image: image_hash,
                tables: self.tables_hash,
            });
        }
```

- [ ] **Step 3: Write discriminant tests**

`binding.rs` tests 加（用 `ImageParts` 直接造带指定 hash 的镜像；`w.tables_hash` 是 `pub(crate)`，本 crate 测试可直写）：
```rust
#[test]
fn start_main_rejects_image_table_hash_mismatch() {
    use crate::ecl::image::{EclImage, ImageParts, SubInit, SubKind};
    let image = EclImage::try_from_parts(ImageParts {
        code: vec![0],
        subs: vec![SubInit::new(0, SubKind::Root, vec![])],
        entries: vec![],
        root: Some(0),
        content_hash: 0xAAAA_AAAA,
    })
    .unwrap();
    let mut w = World::new(0);
    w.tables_hash = 0xBBBB_BBBB;
    assert_eq!(
        w.start_main(&image),
        Err(TaskStartError::TableImageMismatch { image: 0xAAAA_AAAA, tables: 0xBBBB_BBBB })
    );
}

#[test]
fn start_main_allows_matching_hash() {
    use crate::ecl::image::{EclImage, ImageParts, SubInit, SubKind};
    let image = EclImage::try_from_parts(ImageParts {
        code: vec![0],
        subs: vec![SubInit::new(0, SubKind::Root, vec![])],
        entries: vec![],
        root: Some(0),
        content_hash: 0x1234,
    })
    .unwrap();
    let mut w = World::new(0);
    w.tables_hash = 0x1234;
    assert!(w.start_main(&image).is_ok());
}

#[test]
fn start_main_zero_hash_escapes_guard() {
    // 空镜像 hash 0 → 守卫跳过；落到既有 NoRoot（证明未误报 mismatch）。
    let mut w = World::new(0);
    w.tables_hash = 0x9999;
    assert_eq!(w.start_main(&crate::ecl::image::EclImage::empty()), Err(TaskStartError::NoRoot));
}
```

- [ ] **Step 4: Run — test + workspace + 金向量能跑完**

Run:
```bash
cargo test -p stg-core ecl::binding 2>&1 | tail -6
cargo test --workspace 2>&1 | tail -4
cargo run -q -p stg-harness -- golden --out /tmp/golden_C3.txt >/dev/null && echo "golden ran (scene2 guard passed: compile+World both bind TABLES_V0)"
```
Expected: 守卫三腿 PASS；workspace 全绿；golden 跑完（scene 2 编译与 World 同绑 `TABLES_V0`，hash 相等，守卫放行）。

- [ ] **Step 5: fmt/clippy + Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
git add crates/stg-core/src/ecl/binding.rs
git commit -m "feat(ecl): coherence guard rejects image/table content_hash mismatch

start_main checks EclImage.content_hash == World.tables_hash (0 on either
side = unbound, skip). P4-b deterministic Err, once at startup, not per-frame.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 组 D — join 防迷路 + consts 分组 + 端到端自证 + 销债

### Task D1: `consts.rs` 分组 `ENGINE_STRUCTURAL` / `TABLE_SYMBOLS`

**Files:**
- Modify: `crates/stg-core/src/consts.rs`（`engine_consts!` 宏改双 section；三个 const 列表）
- Test: `crates/stg-core/src/consts.rs`（tests）

**Interfaces:**
- Produces：`pub const ENGINE_STRUCTURAL: &[EngineConst]`（①）、`pub const TABLE_SYMBOLS: &[EngineConst]`（②）、`pub const ENGINE_CONSTS: &[EngineConst]`（①⧺②，注入用，callers 不变）；各 `pub const NAME` 不变

- [ ] **Step 1: 宏改双 section**

把 `consts.rs` 的 `macro_rules! engine_consts` + 其唯一调用替换为：
```rust
macro_rules! engine_consts {
    (
        structural { $( $sname:ident : $sty:ty as $sk:ident = $sval:expr ; )* }
        table_symbols { $( $tname:ident : $tty:ty as $tk:ident = $tval:expr ; )* }
    ) => {
        $( pub const $sname: $sty = $sval; )*
        $( pub const $tname: $tty = $tval; )*
        /// ① 引擎结构常量（VM/ABI，与表无关，版本漂移归 `engine_ver`）。
        pub const ENGINE_STRUCTURAL: &[EngineConst] = &[
            $( EngineConst::new(stringify!($sname), engine_consts!(@ty $sk), $sval as i32), )*
        ];
        /// ② 表符号词汇（数据表某些行的名字；join 校验对象；乙案将来搬进表符号段）。
        pub const TABLE_SYMBOLS: &[EngineConst] = &[
            $( EngineConst::new(stringify!($tname), engine_consts!(@ty $tk), $tval as i32), )*
        ];
        /// 全部脚本可见引擎常量（编译器注入 = ①⧺②）。callers 用此名，签名不变。
        pub const ENGINE_CONSTS: &[EngineConst] = &[
            $( EngineConst::new(stringify!($sname), engine_consts!(@ty $sk), $sval as i32), )*
            $( EngineConst::new(stringify!($tname), engine_consts!(@ty $tk), $tval as i32), )*
        ];
    };
    // v0 限制：`$val as i32` 要求 `$rust_ty` 为原生整数（现均 u16）。fx/angle 分支只标
    // `EclValueType`，不改求值——真登记 fx/angle 常量须 `$val` 已是 raw 整数（见 C14）。
    (@ty int)   => { EclValueType::Int };
    (@ty fx)    => { EclValueType::Fx };
    (@ty angle) => { EclValueType::Angle };
}

engine_consts! {
    structural {
        GVAR_RANK:           u16 as int = 0;
        GLOBALS_SYS_SEGMENT: u16 as int = 16;
    }
    table_symbols {
        //  ② appearance 行名（值 = appearances 索引；join 校验 + FM2 防错序的锚）
        APPEARANCE_SMALL:  u16 as int = 0;
        APPEARANCE_MEDIUM: u16 as int = 1;
        APPEARANCE_LARGE:  u16 as int = 2;
        APPEARANCE_STAR:   u16 as int = 3;
    }
}
```

- [ ] **Step 2: Write test（分组正确 + 并集 = 注入集）**

`consts.rs` tests 替换/追加：
```rust
#[test]
fn consts_split_into_structural_and_table_symbols() {
    let has = |list: &[EngineConst], name: &str| list.iter().any(|c| c.name == name);
    assert!(has(ENGINE_STRUCTURAL, "GVAR_RANK") && has(ENGINE_STRUCTURAL, "GLOBALS_SYS_SEGMENT"));
    assert!(has(TABLE_SYMBOLS, "APPEARANCE_SMALL") && has(TABLE_SYMBOLS, "APPEARANCE_STAR"));
    assert!(!has(TABLE_SYMBOLS, "GVAR_RANK"), "结构常量不入 ②");
    assert!(!has(ENGINE_STRUCTURAL, "APPEARANCE_STAR"), "表符号不入 ①");
    // ENGINE_CONSTS = ①⧺② 且注入面不变
    assert_eq!(ENGINE_CONSTS.len(), ENGINE_STRUCTURAL.len() + TABLE_SYMBOLS.len());
    for c in ENGINE_STRUCTURAL.iter().chain(TABLE_SYMBOLS) {
        assert!(ENGINE_CONSTS.iter().any(|e| e.name == c.name && e.value == c.value));
    }
    assert_eq!(APPEARANCE_STAR, 3u16); // pub const 原型不变
}
```

- [ ] **Step 3: Run + Commit**

Run: `cargo test -p stg-core consts:: 2>&1 | tail -4 && cargo test --workspace 2>&1 | tail -3`
Expected: 全 PASS（`ENGINE_CONSTS` 注入面未变，编译器常量注入回归绿）。
```bash
cargo fmt --all
git add crates/stg-core/src/consts.rs
git commit -m "refactor(consts): split engine consts into structural + table-symbol groups

ENGINE_STRUCTURAL (①, engine ABI) / TABLE_SYMBOLS (②, table row names) /
ENGINE_CONSTS (①⧺②, injection surface unchanged). Enables data-driven join validate.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task D2: `validate()` join 腿 + coverage 断言 + 角色半径负向腿（B14）

**Files:**
- Modify: `crates/stg-core/src/tables.rs`（`validate()` 加 join 腿）
- Test: `crates/stg-core/src/tables.rs`（tests）

**Interfaces:**
- Consumes：`crate::consts::TABLE_SYMBOLS`（D1）；`self.appearances`

- [ ] **Step 1: `validate()` 加 join 腿**

`tables.rs` `WorldTables::validate()` 末尾（`Ok`/`true` 之前）插入：
```rust
        // join 校验（防 FM1）：每个 ② 表符号 id 必须是 appearances 的合法行。v1 全部 ②
        // 都是 appearance 索引；将来 ② 长出 item 符号时按 tag 分流（见 spec/follow-ups）。
        for c in crate::consts::TABLE_SYMBOLS {
            if (c.value as usize) >= self.appearances.len() {
                return false;
            }
        }
```

- [ ] **Step 2: Write discriminant + coverage + B14 tests**

```rust
#[test]
fn validate_rejects_table_symbol_without_appearance_row() {
    // FM1：appearances 长度不覆盖 APPEARANCE_STAR(3) → 拒。
    let mut t = build_tables_v0();
    t.appearances = Box::new([AppearanceCfg { radius: Fx::from_int(3), sprite: 0 }]); // len 1
    assert!(!t.validate(), "② 符号 id 越出 appearances → join 拒（FM1）");
}

#[test]
fn builtin_appearances_exactly_cover_table_symbols() {
    use crate::consts::TABLE_SYMBOLS;
    assert_eq!(
        TABLES_V0.appearances.len(),
        TABLE_SYMBOLS.len(),
        "内建 appearances 恰覆盖 ② 命名集（无空洞/无缺失）"
    );
    for c in TABLE_SYMBOLS {
        assert!((c.value as usize) < TABLES_V0.appearances.len());
    }
}

#[test]
fn validate_rejects_bad_character_radius() {
    // B14 债：角色 hit/graze 半径越界的负向腿（此前只有正向覆盖）。
    let mut t = build_tables_v0();
    t.characters[0].hit_radius = Fx::from_int(2000); // 超 MAX_ENTITY_RADIUS(1024)
    assert!(!t.validate(), "角色 hit_radius 超上限必须被 validate 拒绝");
    let mut t2 = build_tables_v0();
    t2.characters[0].graze_radius = Fx::from_int(2000);
    assert!(!t2.validate(), "角色 graze_radius 超上限必须被 validate 拒绝");
}
```

- [ ] **Step 3: Run + Commit**

Run: `cargo test -p stg-core tables:: 2>&1 | tail -5`
Expected: 全 PASS。
```bash
cargo fmt --all && cargo clippy -p stg-core --all-targets -- -D warnings 2>&1 | tail -3
git add crates/stg-core/src/tables.rs
git commit -m "feat(tables): join validation + coverage + character-radius negative legs

validate() rejects any ② table-symbol id without an appearances row (FM1);
coverage test pins builtin appearances == TABLE_SYMBOLS; adds B14 character
radius negative legs.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task D3: 端到端文件加载自证 + interval 热路径 debug 兜底（C11③）

**Files:**
- Modify: `crates/stg-harness/src/main.rs`（tests：磁盘 `from_bytes` 端到端）
- Modify: `crates/stg-core/src/world/player.rs`（发弹热路径 `timer % interval` 前加 `debug_assert!`）

**Interfaces:**
- Consumes：`WorldTables::from_bytes`（B1）、`World::new_with_tables`（C2）、`stg_core::step`、`World::checksum()`

- [ ] **Step 1: interval 热路径 debug 兜底（C11③）**

`crates/stg-core/src/world/player.rs` 的 `char0_update_shot`（发弹逻辑里，`timer % shooter.interval` 或等价取模处，用 `grep -n "interval" crates/stg-core/src/world/player.rs` 定位）——在取模前加：
```rust
        debug_assert!(shooter.interval != 0, "interval==0 应被加载期 validate() 挡下（外部表兜底）");
```
（加载期 `validate()` 已拒 `interval==0`；此为 P4-c 对称的引擎内自检兜底。）

- [ ] **Step 2: Write 端到端自证 test（harness）**

`crates/stg-harness/src/main.rs` 的 `#[cfg(test)] mod` 加（`InputFrame` 的中性构造沿用金向量场景一的 idle 输入写法——若字段不同，照 golden scene 1 构造）：
```rust
#[test]
fn table_loaded_from_disk_runs_identically_to_builtin() {
    use stg_core::checksum::Checksum; // `.checksum()` 是 trait 方法，需入作用域
    // 圆环自证：从磁盘 from_bytes 载表 vs include_bytes! 内建表，逐帧校验和一致。
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../stg-core/src/tables/tables_v0.bin");
    let bytes = std::fs::read(path).expect("committed tables_v0.bin exists");
    let loaded = stg_core::tables::WorldTables::from_bytes(&bytes).expect("disk table loads");

    let mut w_disk = stg_core::World::new_with_tables(0xC11, &loaded);
    let mut w_builtin = stg_core::World::new(0xC11);
    let ecl = stg_core::ecl::image::EclImage::empty();
    let input = stg_core::input::InputFrame::default();
    for _ in 0..120 {
        stg_core::step(&mut w_disk, &loaded, &ecl, &input);
        stg_core::step(&mut w_builtin, &stg_core::tables::TABLES_V0, &ecl, &input);
    }
    assert_eq!(
        w_disk.checksum(),
        w_builtin.checksum(),
        "磁盘加载表与内建表逐帧一致——文件加载闭环自证"
    );
}
```
（`InputFrame::default()` 若不存在，用 `stg_core::input::InputFrame` 的中性/全零构造；不按键的 idle 输入即可。）

- [ ] **Step 3: Run + Commit**

Run:
```bash
cargo test --workspace 2>&1 | tail -6
cargo build -p stg-core 2>&1 | tail -3   # 确认 debug_assert 编译
```
Expected: 端到端 test PASS，workspace 全绿。
```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
git add crates/stg-harness/src/main.rs crates/stg-core/src/world/player.rs
git commit -m "test(harness): end-to-end disk table-load self-proof + interval debug guard

Loads tables_v0.bin from disk via from_bytes, runs 120 frames, asserts checksum
== builtin run (file-loading loop closed). Adds interval!=0 hot-path debug_assert.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task D4: 收口文档（PROGRESS / follow-ups / ecl-lang）

**Files:**
- Modify: `PROGRESS.md`（史加一行 + 重写「现在」段）
- Modify: `docs/follow-ups.md`（C11 销账；B14/C11③ 删条；C14 coherence 注更新；乙案/DSL 记为未来）
- Modify: `docs/ecl-lang.md`（加一句"编译绑定表 / content_hash coherence"）

- [ ] **Step 1: `PROGRESS.md`**

「里程碑史」表最上加一行：
```
| 2026-07-21 | **C11 资产管线** | owned WorldTables + 规范字节 from_bytes/to_bytes + 真 content_hash + compile 绑定表 + start_main coherence 守卫 + join 防迷路 |
```
「现在」段**重写**（≤10 行）：位置更新为"C11 资产管线落地——表从规范字节加载、owned 化、content_hash 焊死编译/运行表一致"；「在飞」无；「下一阶段候选」保留 M2 前置刀 / M3 回滚 harness / bomb 玩法刀，并把 C11 从候选里划掉。

- [ ] **Step 2: `docs/follow-ups.md`**

- C11 条：标注**已还**（owned 化 + 文件加载 + `content_hash`/`EclImage.content_hash` 真哈希 + coherence 守卫全部落地），保留**乙案（表自带符号段）**与**文本 DSL** 为未来 modding 扩展点。
- 删 B14 条（角色半径负向腿已补）、C11③ 相关（interval 加载期 validate + 热路径 debug 已做）。
- C14 coherence 注：更新为"C11 已焊死——`compile_for_table` 盖 hash + `start_main` 守卫"。

- [ ] **Step 3: `docs/ecl-lang.md`**

在"引擎常量"节末加一句：`.ecl` 编译现绑定一张表（`compile`/`compile_for_table`），编译产物 `EclImage` 记录该表 `content_hash`；运行时若加载的表与之不符，`start_main` 拒绝启动（`TableImageMismatch`）。

- [ ] **Step 4: 全绿终检 + Commit**

Run:
```bash
cargo test --workspace 2>&1 | tail -3
cargo run -q -p stg-harness -- verify-tables 2>&1 | tail -3
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo tree -p stg-core 2>&1 | grep -iE "godot|rand|libm|chrono|std-time" && echo "FIREWALL BREACH" || echo "firewall intact"
```
Expected: 全 PASS，verify-tables 全绿，fmt/clippy 干净，防火墙 intact。
```bash
git add PROGRESS.md docs/follow-ups.md docs/ecl-lang.md
git commit -m "docs(ecl): close out C11 asset pipeline (PROGRESS + follow-ups + ecl-lang)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 附：全 workspace 终检清单（收口后一次性）

- [ ] `cargo test --workspace` 全绿
- [ ] `cargo run -p stg-harness -- verify-tables` 含 `tables_v0.bin` 逐位一致
- [ ] `cargo run -p stg-harness -- golden` 两段跑完（校验和流因 `tables_hash` 字段合法移动，三平台内部一致；无 committed 基线可破）
- [ ] `cargo fmt --all --check` + `cargo clippy --workspace --all-targets -- -D warnings` 干净
- [ ] `cargo tree -p stg-core` 无 float/时钟/godot/rand 依赖（防火墙）
- [ ] `git status`：`README.md` 仍未暂存（勿动）
