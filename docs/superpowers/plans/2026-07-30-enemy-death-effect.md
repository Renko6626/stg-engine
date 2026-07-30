# 敌人死亡效果可显式调用 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `damage_enemy` 里内联的死亡效果提成可复用函数，掉落改成敌身上的可变计数，脚本得到
`drop_clear` / `drop_add` / `drop_items` / `die` 四个能力，并给死亡补上强制加分。

**Architecture:** 三步走。① 掉落从"生成时定死的表索引"迁成"敌身上按类型计数的可变状态"
（纯重构，行为等价）；② 把死亡效果提成 `kill_enemy()` 并补加分（行为变更）；
③ 四个 syscall 接线，`die()` 由编译器降低成 `SYS + OP_KILL_SELF` 两条指令（零 VM 改动）。

**Tech Stack:** Rust 1.94.0（edition 2024）/ `stg-core`（断层线以下，纯整数）/ `stg-ecl-compiler`

**设计依据：** [`docs/superpowers/specs/2026-07-30-enemy-death-effect-design.md`](../specs/2026-07-30-enemy-death-effect-design.md)
（九条人类裁定见其 §3，不得偏离）。

## Global Constraints

- **I1–I7**：`stg-core` 内不得出现 `f32`/`f64`、系统时钟、宿主 RNG、`HashMap`/`HashSet`。
- **P4 三铁律**：(a) 资源耗尽→确定性降级不 panic + 计数；(b) 调用方违约→安全结果 + 计数；
  (c) 引擎自身 bug→debug 帧内断言。
- **P1**：调用方**永不直接触碰池内存**，只走安全读/写 API。新 syscall 一律经 `WorldBody` 的
  handle 写 API，不在 `syscall.rs` 里直写 `ctx.body.enemies.xxx[i]`。
- **九条人类裁定**（spec §3）中会咬人的四条：
  - **D-3**：`drop_items()` 吐完**不清空**计数 → `drop_items(); die();` = **双份**掉落。
  - **D-4**：`die()` **立即终止**调用它的任务。
  - **D-5**：死亡效果**强制加分**（`enemies.score[e]` → `players[0].score`）。
  - **D-9**：dying 敌**当帧仍参与碰撞**，不特殊处理。
- **syscall 号表 append-only**：54-57 已占（ECL 复刻刀），本刀占 **58/59/60/61**。
- **`ENGINE_VER` 3 → 4**（号表变更），在 T3 一次性 bump；`step.rs` 的 `engine_ver_anchored`
  锚点测试（`step.rs:2168`）同步。
- **金向量漂移分三段记账**（重要，每个任务结束都要对拍并归因）：
  - T1 漂 = **仅池字段布局**（掉落行为等价，靠 T1 的特征化测试独立证明）
  - T2 漂 = **死亡加分**（行为）
  - T3 / T4 **逐字节不变**（纯新增 + 纯文档）
- 每个任务结束前必须全绿：
  ```bash
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
- **commit 结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **分支**：`feat/enemy-death-effect`（从 `main` 开）。

### 金向量对拍的正确做法

仓库**无 committed 金向量基线**（CLAUDE.md「金向量闸门的能力边界」）——`determinism-gate`
只把三平台的流互相比。所以本地对拍只能靠 worktree 副本：

```bash
git worktree add /tmp/edf-base <本任务的 base commit>
(cd /tmp/edf-base && cargo run -q -p stg-harness -- golden --out /tmp/g-base.txt)
cargo run -q -p stg-harness -- golden --out /tmp/g-head.txt
diff -q /tmp/g-base.txt /tmp/g-head.txt   # 按各任务的预期判断"该不该差"
git worktree remove /tmp/edf-base
```

**绝不**在主 checkout 上 `git stash` 或 `git checkout <file>` 去拿 base——前两刀吃过亏
（`git checkout` 清临时代码时把实现一起撤了）。

## 对 spec 的两处订正（写计划时发现，以本节为准）

1. **`kill_enemy` 用 `hp = hp.min(0)` 而不是 `hp = 0`。** spec §4.2 ① 写的是无条件置 0，
   但那会抹掉 overkill 的可观测性——现有测试 `settle_overkill_two_shots_one_death_event`
   （`settle.rs:376`）依赖"打穿的敌 hp 是负数"这一事实。`min(0)` 对伤害路径是恒等
   （hp 已 ≤0），对 `die()` 路径把满血 boss 压到 0，两个目的都达到。
2. **道具类型常量放 ① `structural` 段，不放 ② `table_symbols` 段。**
   `consts.rs:70-73` 的注释把"道具类型符号"点名为将来该进②段的例子，**但那个猜测是错的**：
   ②段存在的意义是 `tables.rs:319-326` 的 join 校验，用来抓"符号 vs **可加载表行**"的漂移；
   而 `item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT]` 是**定长数组**，类型数编译期冻结、
   不可能随表漂移，没有可抓的漂移。且当前 join 校验硬编码校验对象是 `appearances`
   （注释里"届时按 tag 分流"的分流机制并不存在），把 ITEM_* 放进②会得到一个拿
   `appearances.len()` 去校验道具 id 的假检查。**T3 顺带把那句误导性注释改掉。**

---

## File Structure

| 文件 | 责任 | 任务 |
|---|---|---|
| `crates/stg-core/src/enemy.rs` | 池字段 `drop_table: u16` → `drop_count: [u8; ITEM_TYPE_COUNT]` | T1 |
| `crates/stg-core/src/tables.rs` | 新增 `drop_counts()` 展开助手 | T1 |
| `crates/stg-core/src/world/settle.rs` | `spill_drops()`（T1）/ `kill_enemy()`（T2）；`damage_enemy` 瘦身 | T1/T2 |
| `crates/stg-core/src/ecl/syscall.rs` | `sys_spawn_enemy` 展开表（T1）；四个新 syscall（T3） | T1/T3 |
| `crates/stg-core/src/world.rs` | 四个 handle 写 API（P1） | T3 |
| `crates/stg-core/src/consts.rs` | 五个道具类型常量入 ① 段 + 改②段误导注释 | T3 |
| `crates/stg-core/src/lib.rs` `step.rs` | `ENGINE_VER` 3→4 + 锚点 | T3 |
| `crates/stg-ecl-compiler/src/lang/builtins.rs` | 四个表层内建 | T3 |
| `crates/stg-ecl-compiler/src/lang/codegen.rs` | `die()` 的两指令降低 | T3 |
| 其余 ~15 处 `EnemyInit {` 调用点 | 换字段 | T1 |
| `docs/*` `PROGRESS.md` | 文档收口 | T4 |

---

### Task 1: 掉落迁成敌身可变计数（纯重构，行为等价）

**Files:**
- Modify: `crates/stg-core/src/enemy.rs`（池字段 + 测试助手）
- Modify: `crates/stg-core/src/tables.rs`（新增 `drop_counts`）
- Modify: `crates/stg-core/src/world/settle.rs`（`spill_drops` + `damage_enemy` 改用它 + B11 测试搬迁 + 新特征化测试）
- Modify: `crates/stg-core/src/ecl/syscall.rs:654`（`sys_spawn_enemy` 展开表号）
- Modify: 其余 `EnemyInit {` 调用点（完整清单见 Step 4）

**Interfaces:**
- Consumes: `crate::items::ITEM_TYPE_COUNT`（= 5）、`WorldTables.drop_tables`、`WorldBody::spawn_drop`
- Produces:
  - `EnemyPool.drop_count: [[u8; ITEM_TYPE_COUNT]; 256]`（`EnemyInit.drop_count: [u8; ITEM_TYPE_COUNT]`）
  - `pub fn stg_core::tables::drop_counts(tables: &WorldTables, table: u16) -> ([u8; ITEM_TYPE_COUNT], bool)`
  - `pub(crate) fn WorldBody::spill_drops(&mut self, e: usize, tables: &WorldTables)`

- [ ] **Step 1: 写特征化测试，钉住重构前的掉落行为**

这一步的目的特殊：**先钉住现状，再重构**。掉落经 `spawn_drop` 消耗世界 RNG（散布速度），
所以"行为等价"不只是"掉了几颗什么"，还包括 **RNG 消耗的次数与顺序**。断言逐颗的
`(item_type, vx, vy)` 就同时钉住了这三者。

加到 `crates/stg-core/src/world/settle.rs` 的 `mod tests`：

```rust
    /// **特征化测试**（T1 重构的安全网）：敌死掉落的逐颗 `(type, vx, vy)` 全序列。
    /// `vx`/`vy` 来自 `spawn_drop` 的世界 RNG 散布，故本测试同时钉住"掉了什么"
    /// **与** "RNG 被消耗了几次、按什么顺序"——掉落迁成按类型计数后这三者都必须不变。
    /// 期望值是**重构前实测**填入的（见计划 T1 Step 2）。
    #[test]
    fn enemy_death_drop_sequence_is_pinned() {
        let mut w = crate::step::World::new(1);
        let e = w.body.create_enemy(enemy_with_drop_table(1));
        let ei = w.body.enemies.get(e).unwrap();
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei],
            y: w.body.enemies.y[ei],
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 99,
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);

        let got: Vec<(u8, i32, i32)> = w
            .body
            .items
            .iter_alive()
            .map(|i| {
                (
                    w.body.items.item_type[i],
                    w.body.items.vx[i].raw(),
                    w.body.items.vy[i].raw(),
                )
            })
            .collect();
        // 掉落表 1 = [(ITEM_POWER,2),(ITEM_POINT,1)]；POWER=0 < POINT=1，
        // 表序恰好就是类型升序 —— 这正是"改成按类型计数后顺序不漂"的原因。
        assert_eq!(got, vec![/* Step 2 实测填入三元组 */]);
    }
```

`enemy_with_drop_table` 是本步要写的助手（放同一个 `mod tests`，`drop_table` 参数在
Step 4 之后改成 `drop_count`）：

```rust
    /// 全字段 EnemyInit（exhaustive）：位置固定 (0,80)、hp=1、其余惰性。
    fn enemy_with_drop_table(table: u16) -> crate::enemy::EnemyInit {
        crate::enemy::EnemyInit {
            x: Fx::ZERO,
            y: Fx::from_int(80),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            mv_from_x: Fx::ZERO,
            mv_from_y: Fx::ZERO,
            mv_to_x: Fx::ZERO,
            mv_to_y: Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp: 1,
            hp_max: 1,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: table,
            score: 100,
        }
    }
```

- [ ] **Step 2: 跑它，把实测值填进断言，确认它在重构前是绿的**

```bash
cargo test -p stg-core enemy_death_drop_sequence_is_pinned -- --nocapture
```

第一次必然红（`vec![]` vs 三个元素）。**从失败输出里抄出 `left` 的真值**填进 `vec![...]`，
再跑一次必须**绿**。这条绿是本任务后续所有改动的基准线——它现在描述的是**重构前**的行为。

> 若第一次跑出来不是恰好 3 颗（2 POWER + 1 POINT），**停下来报告**：说明我对掉落表 1 的
> 理解有误，后面的等价性论证要重做。

- [ ] **Step 3: 写 `tables::drop_counts` 助手 + 它的测试**

加到 `crates/stg-core/src/tables.rs`（放在 `build_tables_v0` 之后、`mod tests` 之前）：

```rust
/// 掉落表号 → 逐类型计数（`drop_table` 从"存储状态"退化成"生成参数"的展开口，
/// 敌人死亡效果刀 2026-07-30）。
///
/// 越界表号 → 全零 + `false`；调用方据此计 `contract_viol`（P4-b，原先这条检查在
/// `settle::damage_enemy` 里，随掉落状态一起前移到生成时）。
/// 同一张表里同类型多条目**累加**（`saturating_add`：表已过 `validate`，理论上不该溢出，
/// 但不许在 debug 下 panic）。
pub fn drop_counts(tables: &WorldTables, table: u16) -> ([u8; ITEM_TYPE_COUNT], bool) {
    let mut out = [0u8; ITEM_TYPE_COUNT];
    let Some(rows) = tables.drop_tables.get(table as usize) else {
        return (out, false);
    };
    for &(ty, n) in rows.iter() {
        // `validate` 已保证 `ty < ITEM_TYPE_COUNT`；`get_mut` 是防御性的，不 panic。
        if let Some(slot) = out.get_mut(ty as usize) {
            *slot = slot.saturating_add(n);
        }
    }
    (out, true)
}
```

测试加到 `tables.rs` 的 `mod tests`：

```rust
    /// `drop_counts`：内建表 1 展开成逐类型计数；越界表号降级成全零 + false（P4-b）。
    #[test]
    fn drop_counts_expands_table_and_degrades_out_of_range() {
        let (c1, ok1) = drop_counts(&TABLES_V0, 1);
        assert!(ok1);
        assert_eq!(c1[crate::items::ITEM_POWER as usize], 2);
        assert_eq!(c1[crate::items::ITEM_POINT as usize], 1);
        assert_eq!(c1.iter().map(|&n| n as u32).sum::<u32>(), 3, "别的类型必须是 0");

        let (c0, ok0) = drop_counts(&TABLES_V0, 0);
        assert!(ok0, "表 0 是合法的空表，不是越界");
        assert_eq!(c0, [0u8; crate::items::ITEM_TYPE_COUNT]);

        let bad = TABLES_V0.drop_tables.len() as u16 + 9;
        let (cb, okb) = drop_counts(&TABLES_V0, bad);
        assert!(!okb, "越界表号必须报 false");
        assert_eq!(cb, [0u8; crate::items::ITEM_TYPE_COUNT]);
    }
```

跑：`cargo test -p stg-core drop_counts_expands_table` → 绿。

- [ ] **Step 4: 换池字段 + 扫平所有调用点**

`crates/stg-core/src/enemy.rs` 的 `define_pool!`：

```rust
        main_task: u32, death_script: u16,
        // 掉落计数（敌人死亡效果刀 2026-07-30）：**逐道具类型的待掉落颗数**，敌身上的
        // 可变状态。此前是 `drop_table: u16`（生成时定死的表索引）——改成计数后脚本可以
        // 增量配置（`drop_clear`/`drop_add`），且"撒掉落"与"死亡"得以解耦（`drop_items`）。
        // `drop_table` 未消失，只是退化成 `spawn_enemy` 的**生成参数**：由
        // `tables::drop_counts` 在建敌时展开进本字段，此后无人读表号。
        // 撒的顺序是**类型升序**（I4），见 `world::settle::spill_drops`。
        drop_count: [u8; crate::items::ITEM_TYPE_COUNT],
        score: u16
```

然后编译，跟着报错逐个改。**完整调用点清单**（`EnemyInit {` 字面量，改
`drop_table: X,` → `drop_count: <展开>,`）：

| 文件:行 | 处置 |
|---|---|
| `crates/stg-core/src/enemy.rs:41` | `drop_count: [0; crate::items::ITEM_TYPE_COUNT]` |
| `crates/stg-core/src/step.rs:2203` | 同上（原 `drop_table: 0`） |
| `crates/stg-core/src/world.rs:1054` | 按原表号：0 → 全零 |
| `crates/stg-core/src/world.rs:1150` | 同上 |
| `crates/stg-core/src/world.rs:1355` | 同上 |
| `crates/stg-core/src/world.rs:1476` | 同上 |
| `crates/stg-core/src/world/settle.rs:425` | **B11 测试**，见 Step 5 |
| `crates/stg-core/src/world/settle.rs:708` | 原 `drop_table: 1` → `crate::tables::drop_counts(&crate::tables::TABLES_V0, 1).0` |
| `crates/stg-core/src/ecl/syscall.rs:654` | **生成路径**，见下方 |
| `crates/stg-core/src/ecl/syscall.rs:1088` | 测试，按原表号展开 |
| `crates/stg-core/src/ecl/vm.rs:1694` | 测试（D9 那组），按原表号展开 |
| `crates/stg-ecl-compiler/src/lang/codegen.rs:1484` | 测试，按原表号展开 |
| `crates/stg-harness/src/main.rs:218` | 原 `drop_table: 1` → 展开 |
| `crates/stg-harness/src/main.rs:439` | 原 `drop_table: 1` → 展开 |
| `crates/stg-harness/src/main.rs:939` | 原 `drop_table: 0` → 全零 |
| `crates/stg-harness/src/main.rs:1061` | 原 `drop_table: 0` → 全零 |

> 上表是写计划时 `grep -rn "EnemyInit {" --include=*.rs` 的结果。**以编译器报错为准**——
> 若有遗漏或行号漂了，跟着 `error[E0063]: missing field` 走即可（`Init` 是 exhaustive，
> 漏一个都编不过，这正是它的用处）。

`sys_spawn_enemy`（`syscall.rs`）的生成路径——**P4-b 检查从 settle 前移到这里**：

```rust
    // 掉落表号在**生成时**展开成逐类型计数（此前存表号、死时才查表）。
    // P4-b：越界表号 → 视同空表 + 计数（原检查在 `settle::damage_enemy`，随状态前移）。
    let (drop_count, table_ok) = crate::tables::drop_counts(ctx.tables, drop_table as u16);
    if !table_ok {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
    }
```

并把 `init` 里的 `drop_table: drop_table as u16,` 换成 `drop_count,`。

> `drop_table` 是 `i32`（`pop` 的产物）。负数 `as u16` 会回绕成大正数，而大正数必然越界
> → 走 `table_ok == false` 的降级，**不 panic**。这条要在下面 Step 7 的测试里钉住。

- [ ] **Step 5: `damage_enemy` 改用 `spill_drops` + B11 测试搬迁**

`settle.rs` 里 `damage_enemy` 内的掉落段（原 `settle.rs:43-54`，即"掉落直接分配"注释起到
`}` 止）整段替换为一行 `self.spill_drops(e, tables);`，并在 `impl WorldBody` 里新增：

```rust
    /// 把敌身上的掉落计数原位撒出去——**只撒**：不清零、不加分、不发事件。
    ///
    /// 顺序是**类型升序**（I4）：这也是"掉落从表迁成计数后金向量不漂"的依据——内建掉落表 1
    /// 是 `[(ITEM_POWER,2),(ITEM_POINT,1)]` 而 `ITEM_POWER=0 < ITEM_POINT=1`，表序恰好
    /// 就是类型升序，故 `spawn_drop` 的 RNG 消耗顺序与迁移前逐字相同
    /// （由 `enemy_death_drop_sequence_is_pinned` 特征化测试押运）。
    ///
    /// 两个调用方：`kill_enemy`（死亡效果的一部分）/ `SYS_DROP_ITEMS`（脚本显式撒）。
    /// **不清零**是人类裁定（spec D-3）：故 `drop_items(); die();` 会掉两份，作者自负。
    pub(crate) fn spill_drops(&mut self, e: usize, tables: &WorldTables) {
        let (ex, ey) = (self.enemies.x[e], self.enemies.y[e]);
        for ty in 0..crate::items::ITEM_TYPE_COUNT {
            for _ in 0..self.enemies.drop_count[e][ty] {
                self.spawn_drop(ex, ey, ty as u8, tables);
            }
        }
    }
```

**B11 测试搬迁**：`settle_out_of_range_drop_table_degrades_to_empty`（`settle.rs:421`）
测的是"死时查表越界"，而检查已前移到生成时。把它**改写成测生成路径**并改名，移到
`syscall.rs` 的 `mod tests`（因为现在检查住在 `sys_spawn_enemy` 里）：

```rust
    /// P4-b：`spawn_enemy` 的越界 `drop_table` → 视同空表 + 计 contract_viol，
    /// 敌照建、不 panic、死时不掉道具（原 B11，随掉落状态从 settle 前移到生成时）。
    /// 负数表号同样走这条（`as u16` 回绕成大正数 → 仍越界）。
    #[test]
    fn spawn_enemy_out_of_range_drop_table_degrades_to_empty() {
        // 用 `dispatch(SYS_SPAWN_ENEMY, ...)` 走真派发路径，照本模块既有 syscall 测试
        // 的构造惯例（见 `sys_spawn_enemy` 那组既有测试）。
        // 断言：
        //   handle >= 0                                  —— 敌照建
        //   diag.contract_viol == before + 1             —— 计数
        //   enemies.drop_count[i] == [0; ITEM_TYPE_COUNT] —— 视同空表
        // 再对**负数**表号重跑一遍同样三条。
    }
```

> 具体的世界/镜像构造**照抄 `syscall.rs` 里既有的 `sys_spawn_enemy` 测试**。

- [ ] **Step 6: 跑特征化测试——必须仍然绿**

```bash
cargo test -p stg-core enemy_death_drop_sequence_is_pinned
```

**这是本任务的核心判据。** 期望值是重构前实测的，现在走的是按类型计数的新路径。
绿 = 掉落内容、顺序、RNG 消耗三者全部等价。

**红了怎么办**：不要改期望值去迁就实现。先查是不是 `spill_drops` 的类型升序与原表序不一致
（若将来有掉落表的条目顺序不是类型升序，这个等价性就不成立了——那要在报告里明确说出来，
并把它记成一条 follow-up，而不是偷偷改断言）。

- [ ] **Step 7: 跑 Step 5 搬迁后的 B11 测试**

```bash
cargo test -p stg-core spawn_enemy_out_of_range_drop_table
```

两条腿都要绿：越界正数表号、负数表号。

> 本步**不再**另加"计数饱和"的测试——写计划时想加一条，但在 T1 阶段
> `add_enemy_drop` 还不存在，测试只能自己手写 `saturating_add`，那是在测标准库而不是测我们的
> 契约。饱和语义归 T3 的 `drop_add_clamps_and_saturates_n`（那时才有真正的被测对象）。

- [ ] **Step 8: 全绿 + 金向量归因**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

金向量对拍（做法见 Global Constraints）：**预期 base 与 head 不同**。这是本任务唯一允许的
漂移，且原因**必须**是池字段布局（`drop_table: u16` 换成 `[u8;5]` 改变了 `Checksum` 的
字节流）。掉落行为等价由 Step 6 的特征化测试独立证明——**两者是互相独立的证据，缺一不可**：
校验和变了不代表行为变了，特征化测试绿也不代表校验和该变。报告里把这两条分开写。

- [ ] **Step 9: 提交**

```bash
git commit -m "$(cat <<'EOF'
refactor(core): 掉落从"生成时定死的表索引"迁成"敌身上按类型计数的可变状态"

EnemyPool 的 drop_table:u16 → drop_count:[u8; ITEM_TYPE_COUNT]。drop_table 没消失,
退化成 spawn_enemy 的生成参数——由新的 tables::drop_counts() 在建敌时展开,此后无人读表号。
这是"脚本能增量配置掉落 + 撒掉落与死亡解耦"的前置(四个 syscall 在下一刀)。

掉落段从 damage_enemy 提成 spill_drops(),按**类型升序**撒(I4)。

**行为等价**由新的特征化测试押运:enemy_death_drop_sequence_is_pinned 断言逐颗
(type, vx, vy) 全序列——vx/vy 来自 spawn_drop 的世界 RNG 散布,故它同时钉住"掉了什么"
与"RNG 被消耗了几次、按什么顺序"。期望值是重构**前**实测填入的,重构后仍绿。
等价成立的原因:内建表 1 是 [(POWER,2),(POINT,1)] 而 POWER=0<POINT=1,表序恰好就是类型升序。

越界 drop_table 的 P4-b 检查随状态从 settle 前移到 sys_spawn_enemy(原 B11 测试同步搬迁
并补负数表号一腿——负数 as u16 回绕成大正数,仍走越界降级不 panic)。

**金向量漂移:仅池字段布局**([u8;5] 换掉 u16 改变了 Checksum 字节流),行为未变。
两条证据独立:校验和变了不代表行为变了,特征化测试绿也不代表校验和该变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 死亡效果提成 `kill_enemy()` + 强制加分

**Files:**
- Modify: `crates/stg-core/src/world/settle.rs`（`kill_enemy` 提取 + `damage_enemy` 瘦身 + 测试）

**Interfaces:**
- Consumes: `spill_drops`（T1）、`ENEMY_DYING`、`push_event`、`emit_req`、`players[0].score`
- Produces: `pub(crate) fn WorldBody::kill_enemy(&mut self, e: usize, tables: &WorldTables)`

- [ ] **Step 1: 写失败测试（三条，两条是判别腿）**

加到 `settle.rs` 的 `mod tests`：

```rust
    /// D-5：敌死**强制加分**——`enemies.score[e]` 记进自机 0 的分数。
    /// **判别腿：测试内一颗道具都不拾取**。此前敌人的 score 是纯装饰字段（只塞进
    /// EVT_ENEMY_DIED 供表现层显示），打死敌人的全部收益来自掉落被 credit_item 入账。
    /// 若测试里让自机拾到了道具，就分不清这分是敌人加的还是道具加的——D9 那刀正是
    /// 在这里栽过（"score 未变"断言因分数走 credit_item 而失效，见 vm.rs 的订正注释）。
    #[test]
    fn enemy_death_credits_its_score_bonus() {
        // 敌在 (0,80)、score=100、**drop_count 全零**（不掉道具，彻底断开道具那条路）。
        // 自机在默认出生点，远离敌与任何道具。
        // 打死它 → players[0].score == before + 100。
    }

    /// 判别腿：**D9 自然退场仍是静默的**——不加分、不掉落。
    /// 防实现者把 kill_enemy 顺手挂到 vm 的 Exec::End 上。
    #[test]
    fn d9_silent_exit_credits_no_score_and_drops_nothing() {
        // 直接置 flags |= ENEMY_DYING（模拟 D9 自燃的效果）后跑一整帧 step，
        // 断言 score 未变、items 为空。
        // **注意**：不要调 kill_enemy——那正是本测试要证明"没被调用"的东西。
    }

    /// 幂等：`kill_enemy` 对已 dying 的敌是 no-op（掉落与加分各只发生一次）。
    #[test]
    fn kill_enemy_is_idempotent() {
        // drop_count = 表1 展开（3 颗）、score=100。
        // 连调两次 kill_enemy → items 恰 3 颗、score 恰 +100。
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core -- credits_its_score kill_enemy_is_idempotent silent_exit_credits`
Expected: 第一条 FAIL（分数没加——这正是缺口）；第三条 FAIL（`kill_enemy` 不存在，编译错）。
第二条此刻应当已经绿（现状就是静默）——**它是反向腿，本来就该一直绿**，实现后必须仍绿。

- [ ] **Step 3: 实现**

`settle.rs` 里把 `damage_enemy` 的整个 `if self.enemies.hp[e] <= 0 { ... }` 块体搬进新函数，
并在原处改成调用：

```rust
    /// 敌人的**完整死亡效果**——死亡路径的唯一实现处。
    ///
    /// **幂等**：已 `ENEMY_DYING` 即直接返回（与 settle 趟二的 overkill 门禁同构）。
    /// 顺序：hp 下钳 → 标 dying → 撒掉落 → 记分 → 事件 → 死亡特效请求。
    ///
    /// 两个调用方：`damage_enemy` 的 `hp<=0` 分支（被自机打死）/ `SYS_DIE`（脚本显式）。
    /// **注意 D9 自燃不走这里**——主协程返回是"静默退场"，不掉道具不加分不发事件
    /// （`ecl::vm::run_tasks` 的 `Exec::End` 分支只置 `ENEMY_DYING`）。三条路径的差异是
    /// 脚本作者最容易搞混的一处，见 `docs/ecl-lang.md`。
    ///
    /// **只标记不回收**——槽要活到相位 8 供表现层读；相位 9 cleanup 收尸。
    pub(crate) fn kill_enemy(&mut self, e: usize, tables: &WorldTables) {
        if self.enemies.flags[e] & ENEMY_DYING != 0 {
            return; // 幂等
        }
        // `min(0)` 而非置 0：伤害路径 hp 已 ≤0（保留 overkill 的负值可观测性，
        // `settle_overkill_two_shots_one_death_event` 依赖它）；`die()` 路径可能打在
        // 满血 boss 上，压到 0 才不会让 HUD 当帧显示"满血的死人"。
        self.enemies.hp[e] = self.enemies.hp[e].min(0);
        self.enemies.flags[e] |= ENEMY_DYING;
        self.spill_drops(e, tables);
        // **强制加分**（人类裁定 D-5，2026-07-30）：此前 `enemies.score` 是纯装饰字段——
        // 只被塞进 `EVT_ENEMY_DIED.data[0]` 与 `REQ_ENEMY_DEATH` 供表现层显示，打死敌人的
        // 全部收益来自掉落被 `credit_item` 入账。记自机 0，与 `SYS_ADD_SCORE` 同口径。
        let bonus = self.enemies.score[e] as u64;
        self.players[0].score = self.players[0].score.saturating_add(bonus);
        let ev = Event {
            kind: crate::events::EVT_ENEMY_DIED,
            a_index: e as u16,
            a_gen: self.enemies.generation[e],
            x: self.enemies.x[e],
            y: self.enemies.y[e],
            data: [
                self.enemies.score[e] as i32,
                self.enemies.death_script[e] as i32,
            ],
        };
        self.push_event(ev);
        // 死亡特效请求（蓝图 §207 机械产出者；args 约定见 `crate::reqs` 模块文档）。
        self.emit_req(
            crate::consts::REQ_ENEMY_DEATH,
            [
                self.enemies.x[e].raw(),
                self.enemies.y[e].raw(),
                self.enemies.sprite[e] as i32,
                self.enemies.score[e] as i32,
                0,
                0,
            ],
        );
    }
```

`damage_enemy` 尾部相应变成：

```rust
        if self.enemies.hp[e] <= 0 {
            self.kill_enemy(e, tables);
        }
```

- [ ] **Step 4: 跑测试确认三条全过 + 既有测试**

```bash
cargo test -p stg-core settle
cargo test -p stg-core spell   # 破卡三路 OR 依赖 ENEMY_DYING，确认没被打扰
```

- [ ] **Step 5: 全绿 + 金向量归因**

金向量对拍：**预期 base 与 head 不同**，原因是**死亡加分**（金向量一号场景的敌人是
`score=100` 且确实会死）。这与 T1 的布局漂移是**两个独立原因**，报告里分开写。

- [ ] **Step 6: 提交**

```bash
git commit -m "$(cat <<'EOF'
feat(core): 死亡效果提成 kill_enemy() + 敌死强制加分

damage_enemy 里内联的死亡效果整块提成 kill_enemy(),死亡路径从此只有一处实现。
幂等门禁(已 dying 即返回)与 settle 趟二的 overkill 门禁同构。

**敌死强制加分**(人类裁定):此前 enemies.score 是**纯装饰字段**——只塞进 EVT_ENEMY_DIED
与 REQ_ENEMY_DEATH 供表现层显示,打死敌人的全部收益来自掉落被 credit_item 入账,
spawn_enemy 的 score 参数等于没接线。现在记进自机 0 的分数,与 add_score 同口径饱和。

判别腿两条:①加分测试里**一颗道具都不拾取**(否则分不清是敌人加的还是道具加的——D9 那刀
正是在这里栽过);②D9 自然退场仍静默、不加分不掉落(防有人把 kill_enemy 挂到 Exec::End 上)。

hp 用 min(0) 不是置 0:伤害路径保留 overkill 负值的可观测性(既有测试依赖),
die() 路径(下一刀)把满血 boss 压到 0,免得 HUD 当帧显示"满血的死人"。

**金向量漂移:死亡加分**(行为)。与上一刀的布局漂移是两个独立原因。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 四个 syscall + 表层内建 + `die()` 降低

**Files:**
- Modify: `crates/stg-core/src/world.rs`（四个 handle 写 API，放在 `move_enemy_to`（`world.rs:479`）之后）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（号 58-61 + 派发 + 测试）
- Modify: `crates/stg-core/src/consts.rs`（五个道具常量 + 改②段误导注释）
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER` 3→4）+ `crates/stg-core/src/step.rs:2168`（锚点）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（四个内建 + 两处穷举名单）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`（`die()` 两指令降低）

**Interfaces:**
- Consumes: `spill_drops`（T1）、`kill_enemy`（T2）、`self_enemy_handle`（`syscall.rs:222`）、
  `ops::OP_KILL_SELF`（值 51）、`SubBuilder::raw_emit_op`（`stg-ecl-compiler/src/lib.rs:210`）
- Produces: `SYS_DROP_CLEAR=58` / `SYS_DROP_ADD=59` / `SYS_DROP_ITEMS=60` / `SYS_DIE=61`；
  表层 `drop_clear()` / `drop_add(type, n)` / `drop_items()` / `die()`；
  引擎常量 `ITEM_POWER`/`ITEM_POINT`/`ITEM_LIFE_PIECE`/`ITEM_BOMB_PIECE`/`ITEM_STAR`

- [ ] **Step 1: 写失败测试**

加到 `syscall.rs` 的 `mod tests`。**七条**，其中四条是钉死人类裁定的：

```rust
    /// D-3 判别腿：`drop_items()` 吐完**不清空**计数 → 再死一次会掉**双份**。
    /// 这是人类裁定（spec §3 D-3），不是 bug——将来有人"顺手修好"成清零语义，这条会红。
    #[test]
    fn drop_items_does_not_clear_counts_so_dying_after_drops_twice() {
        // 敌 drop_count = 表1 展开（3 颗）。
        // call(SYS_DROP_ITEMS) → items == 3
        // 再 kill_enemy → items == 6（**不是** 3）
    }

    /// D-4 判别腿：`die()` 立即终止调用它的任务——后续语句不执行。
    ///
    /// **这条必须走真编译产物**（`die()` 的降低是编译器的事，见 Step 7），不能手拼字节码
    /// ——手拼时你自己会记得发 `OP_KILL_SELF`，那就测不到"编译器有没有发它"。
    /// 故本测试住 `crates/stg-ecl-compiler/src/lang/codegen.rs` 的 `mod tests`
    /// （那里有编译 + 跑世界的现成脚手架，见其既有测试）。
    /// 脚本形如：`async sub e() { die(); set_global(21, 777); }`，敌 owner。
    /// 断言：`globals[21] != 777`（后续语句没跑）**且** 敌已 `ENEMY_DYING`（die 本身生效了）
    /// ——两条一起才排除"根本没执行到 die"的假绿。
    #[test]
    fn die_terminates_the_calling_task_immediately() { /* ... */ }

    /// `die()` 走**完整**死亡效果：四件齐。判别腿——只标 dying 不跑效果的实现会红。
    #[test]
    fn die_runs_the_full_death_effect() {
        // 敌 drop_count = 表1 展开（3 颗）、score=100、hp=8888（远高于任何伤害）。
        // 走 dispatch(SYS_DIE, ...)。断言四件：
        //   items.iter_alive().count() == 3                    —— 掉落
        //   players[0].score == before + 100                   —— 加分（测试内不拾取道具）
        //   frame_events 里有一条 EVT_ENEMY_DIED               —— 事件
        //   take_requests() 里有一条 REQ_ENEMY_DEATH           —— 死亡特效
        //   flags & ENEMY_DYING != 0                           —— 标记
    }

    /// `die()` 打**满血** boss → hp 归 0（不是留在 8888）。
    /// 防"HUD 当帧显示满血死人"；这也是 `min(0)` 与"什么都不做"的判别点。
    #[test]
    fn die_on_full_hp_enemy_zeroes_hp() {
        // 敌 hp = hp_max = 8888，走 dispatch(SYS_DIE, ...)。
        // 断言 enemies.hp[i] == 0。
        // **反向腿**（同一测试内）：另造一只敌先被打到 hp = -5（overkill）再 kill_enemy，
        // 断言 hp 仍是 -5 —— 证明用的是 min(0) 而不是无条件置 0
        //（overkill 的负值可观测性是 settle_overkill_two_shots_one_death_event 的依赖）。
    }

    /// P4-b：`drop_add` 坏类型（≥ ITEM_TYPE_COUNT / 负数）→ no-op + contract_viol，无掉落。
    #[test]
    fn drop_add_bad_type_degrades_and_counts() { /* 两条腿：越界正数、负数 */ }

    /// P4-b：`drop_add` 的 n 为负或巨大 → 钳位后饱和，不 panic 不回绕。
    /// `n = -5` → 计数不变（视同 0）；`n = i32::MAX` → 计数封顶 255。
    #[test]
    fn drop_add_clamps_and_saturates_n() { /* ... */ }

    /// misuse：非 enemy-owner 调这四个 → Fault（照 self_enemy_handle 既有口径）。
    #[test]
    fn drop_and_die_syscalls_fault_for_non_enemy_owner() { /* 四个各一腿 */ }
```

> 世界/镜像构造照抄本模块既有 syscall 测试的惯例。

- [ ] **Step 2: 跑测试确认失败**（符号不存在 → 编译错）

- [ ] **Step 3: 四个 handle 写 API（P1）**

加到 `crates/stg-core/src/world.rs`，紧跟 `move_enemy_to`（`world.rs:479`）之后，
照它的 P4 处置形状（悬垂 → `contract_viol` + `STATUS_STALE_HANDLE` + 返回）：

```rust
    /// 清空敌人的待掉落计数（敌人死亡效果刀）。P4-b：悬垂 → no-op + 计数。
    pub fn clear_enemy_drops(&mut self, h: EnemyHandle) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.enemies.drop_count[i] = [0; crate::items::ITEM_TYPE_COUNT];
    }

    /// 给敌人的待掉落计数**增量**加 `n` 颗 `item_type`（敌人死亡效果刀）。
    ///
    /// P4-b 三处：悬垂 → no-op + 计数；`item_type` 越界 → no-op + 计数；
    /// `n` 先钳进 `[0, u8::MAX]` 再对计数 `saturating_add`。**两步都要**——只钳不饱和会在
    /// 计数接近 255 时溢出（debug 下 panic），只饱和不钳则负数 `as u8` 会回绕成大正数。
    /// 本刀不做"减掉落"，故负 `n` 视同 0（要清空用 `clear_enemy_drops`）。
    pub fn add_enemy_drop(&mut self, h: EnemyHandle, item_type: i32, n: i32) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        if item_type < 0 || item_type as usize >= crate::items::ITEM_TYPE_COUNT {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return;
        }
        let add = n.clamp(0, u8::MAX as i32) as u8;
        let slot = &mut self.enemies.drop_count[i][item_type as usize];
        *slot = slot.saturating_add(add);
    }

    /// 脚本显式撒掉落（敌人死亡效果刀）。**不清零、不加分、不发事件**（人类裁定 D-3）。
    /// **不设 dying 门禁**——对已 dying 的敌照撒不误（与 `kill_enemy` 的幂等门禁不同）。
    /// P4-b：悬垂 → no-op + 计数。
    pub fn spill_enemy_drops(&mut self, h: EnemyHandle, tables: &crate::tables::WorldTables) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.spill_drops(i, tables);
    }

    /// 脚本显式触发敌人的完整死亡效果（敌人死亡效果刀）。幂等（已 dying → no-op）。
    /// P4-b：悬垂 → no-op + 计数。
    pub fn kill_enemy_by_handle(&mut self, h: EnemyHandle, tables: &crate::tables::WorldTables) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.kill_enemy(i, tables);
    }
```

- [ ] **Step 4: 号表 + 派发**

`syscall.rs` 号表（5x 族尾部，54-57 已占）：

```rust
/// 清空自身待掉落计数（58；0 参、无返回。敌人死亡效果刀，参照 ZUN ECL 的 `dropClear` 506）。
/// self owner 必须是 ENEMY，否则 Fault（misuse 策略，同 `move_enemy_to`）。
pub const SYS_DROP_CLEAR: u16 = 58;
/// 给自身待掉落计数增量加 `n` 颗 `type`（59；2 参、无返回。参照 ZUN `dropExtra` 507）。
/// **只增不减**是人类裁定——要清空用 `drop_clear()`。坏类型/负 n 的处置见 `add_enemy_drop`。
pub const SYS_DROP_ADD: u16 = 59;
/// 立刻把自身待掉落计数撒出去（60；0 参、无返回。参照 ZUN `dropItems` 509）。
/// **吐完不清空**（人类裁定 D-3，照 ZUN 字面）——故 `drop_items(); die();` 掉**双份**，
/// 作者自负。这条语义有测试钉死（`drop_items_does_not_clear_counts_...`），别"顺手修好"。
pub const SYS_DROP_ITEMS: u16 = 60;
/// 就地阵亡：跑完整死亡效果（61；0 参、无返回。参照 ZUN `die` 561）。
/// **表层 `die()` 降低成本 syscall + `OP_KILL_SELF` 两条指令**（见 codegen），故调用它的
/// 任务立即终止（人类裁定 D-4）。ZUN 的 561 还经 `setDeath`(556) 间接一层——那半留给
/// `death_script` 通电那一刀，届时与 ZUN 完全同构。
pub const SYS_DIE: u16 = 61;
```

派发臂：

```rust
        SYS_DROP_CLEAR => {
            let h = self_enemy_handle(task)?;
            ctx.body.clear_enemy_drops(h);
            Ok(())
        }
        SYS_DROP_ADD => {
            let h = self_enemy_handle(task)?;
            let n = pop(task)?;
            let item_type = pop(task)?;
            ctx.body.add_enemy_drop(h, item_type, n);
            Ok(())
        }
        SYS_DROP_ITEMS => {
            let h = self_enemy_handle(task)?;
            ctx.body.spill_enemy_drops(h, ctx.tables);
            Ok(())
        }
        SYS_DIE => {
            let h = self_enemy_handle(task)?;
            ctx.body.kill_enemy_by_handle(h, ctx.tables);
            Ok(())
        }
```

> **参数弹出顺序是逆序**（照 `sys_move_enemy_to`：先 `pop` 最后一个形参）。`drop_add(type, n)`
> 故先 `pop` 出 `n`、再 `pop` 出 `item_type`。写反了测试会抓到（坏类型那条腿会拿 `n` 当类型）。

- [ ] **Step 5: 五个道具类型常量入 ① 段 + 改②段误导注释**

`consts.rs` 的 `structural { ... }` 里追加（放 `REQ_SCRIPT_BASE` 之后）：

```rust
        //  道具类型编号（`items.rs` 冻结编号；`drop_add` 的第 1 参）。**放①不放②**：
        //  ②段 join 校验（`tables.rs`）是用来抓"符号 vs 可加载表行"漂移的，而
        //  `item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT]` 是定长数组、类型数编译期冻结，
        //  没有可抓的漂移。
        ITEM_POWER:          u8 as int = crate::items::ITEM_POWER;
        ITEM_POINT:          u8 as int = crate::items::ITEM_POINT;
        ITEM_LIFE_PIECE:     u8 as int = crate::items::ITEM_LIFE_PIECE;
        ITEM_BOMB_PIECE:     u8 as int = crate::items::ITEM_BOMB_PIECE;
        ITEM_STAR:           u8 as int = crate::items::ITEM_STAR;
```

> 宏的 v0 限制是"`$val as i32` 要求 `$rust_ty` 为原生整数"（`consts.rs:46-47`）——`u8` 满足。
> 但宏体会生成 `pub const ITEM_POWER: u8 = crate::items::ITEM_POWER;`，与 `items.rs` 的同名
> 常量在**不同模块**，不冲突。若编译器抱怨重复定义，说明 `consts::*` 被 glob 导入到了
> 某处与 `items::*` 打架——那就在报告里说明，改用 `ITEM_TYPE_POWER` 之类的别名。

同时把 `consts.rs:70-73` ②段注释里 **"（如道具类型符号）"** 这个例子删掉，换成一句说明：
道具类型符号经评估**属于①段**（理由同上），②段仍空、机制保留待将来真正的可加载表符号。
`tables.rs:319-321` 的 join 校验注释里同样的例子一并改。

- [ ] **Step 6: 四个表层内建**

`builtins.rs` 照 `add_score`（`builtins.rs:416-424`）的形状加四条：

```rust
    // ── 敌人死亡效果（syscall 58-61；参照 ZUN ECL 506/507/509/561）───────────
    Builtin {
        name: "drop_clear",
        syscall: syscall::SYS_DROP_CLEAR,
        is_op: false,
        params: &[],
        ret: None,
        doc: "清空自身待掉落计数;self 必须是敌",
        param_names: &[],
    },
    Builtin {
        name: "drop_add",
        syscall: syscall::SYS_DROP_ADD,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "自身待掉落计数增量加 n 颗 type(只增不减,要清空用 drop_clear);计数上限 255 饱和",
        param_names: &["type", "n"],
    },
    Builtin {
        name: "drop_items",
        syscall: syscall::SYS_DROP_ITEMS,
        is_op: false,
        params: &[],
        ret: None,
        doc: "立刻撒出自身待掉落计数;**吐完不清空**(故 drop_items();die(); 掉双份);不加分不发死亡事件",
        param_names: &[],
    },
    Builtin {
        name: "die",
        syscall: syscall::SYS_DIE,
        is_op: false,
        params: &[],
        ret: None,
        doc: "就地阵亡:掉落+加分+死亡事件+死亡特效,并**立即终止本任务**(后续语句不执行)",
        param_names: &[],
    },
```

并更新同文件的**两处穷举名单断言**（`lookup_finds_every_documented_builtin_by_name` 的
`names` 数组、`void_builtins_have_none_return_type` 的名单），四个名字都加。

- [ ] **Step 7: `die()` 的两指令降低**

`codegen.rs` 的 `gen_builtin_call` 末尾（`b.sys(bi.syscall)` 之后）追加：

```rust
        } else {
            b.sys(bi.syscall);
            if emits_kill_self_after(bi.name) {
                // `die()` 降低成两条指令：`SYS(SYS_DIE)` 跑死亡效果，`OP_KILL_SELF` 终止本
                // 任务（人类裁定 D-4）。这样做是因为 `syscall::dispatch` 的签名是
                // `Result<(), u8>`，没有"结束本任务"的返回通道——与其给六十个 match arm
                // 换返回类型，不如在这里发第二条指令（零 VM 改动、零 op 表改动）。
                b.raw_emit_op(stg_core::ecl::ops::OP_KILL_SELF);
            }
        }
```

并在 `is_self_bullet_setter`（`codegen.rs:110`）旁边加同款名字键控助手：

```rust
/// 降低后要追发 `OP_KILL_SELF` 的内建（当前仅 `die`）。名字键控，与
/// `is_self_bullet_setter` 同款——集中在一处，免得散落在 `gen_builtin_call` 里。
fn emits_kill_self_after(name: &str) -> bool {
    name == "die"
}
```

- [ ] **Step 8: `ENGINE_VER` 3 → 4**

`crates/stg-core/src/lib.rs` 的 `ENGINE_VER`，理由写进该行上方注释：**syscall 号表变更**
（新增 58-61），与 T2 新增 `SYS_CLEAR_BULLETS` 时 bump 的口径一致。
`step.rs:2168` 的 `engine_ver_anchored` 断言值与说明字符串同步改成 4 与本刀理由。

- [ ] **Step 9: 重跑元数据生成器**

改了 `builtins.rs` 就必须同步，否则两条防漂移测试红：

```bash
cargo run -p stg-harness -- gen-ecl-meta
```

核对 diff 只有这四个新内建的签名（`docs/ecl-lang.md` 生成段 + `ecl-meta.json`），无夹带。
**手写语义节归 T4，本任务不写。**

- [ ] **Step 10: 跑测试确认七条全过**

```bash
cargo test -p stg-core -- drop_items_does_not_clear die_terminates die_runs_the_full \
  die_on_full_hp drop_add_bad_type drop_add_clamps drop_and_die_syscalls_fault
cargo test --workspace
```

- [ ] **Step 11: 全绿 + 金向量对拍**

金向量**必须逐字节相同**——四个 syscall 是纯新增，没有任何现有脚本调用它们；
`ENGINE_VER` 不进校验和（它在握手/回放头里，不在世界状态里）。**差了就是有问题，要查清。**

- [ ] **Step 12: 提交**

```bash
git commit -m "$(cat <<'EOF'
feat(ecl): drop_clear/drop_add/drop_items/die 四个死亡效果 syscall(58-61) + ENGINE_VER 3→4

参照 ZUN ECL 的 dropClear(506)/dropExtra(507)/dropItems(509)/die(561)。四个都经 WorldBody
的 handle 写 API(P1:调用方不直接摸池内存),self owner 必须是敌(misuse → Fault,同 move_to)。

die() 降低成 **SYS(SYS_DIE) + OP_KILL_SELF 两条指令**:dispatch 的签名是 Result<(),u8>,
没有"结束本任务"的返回通道,与其给六十个 arm 换返回类型不如在 codegen 发第二条——
零 VM 改动、零 op 表改动。故 die() 后的语句不执行(人类裁定 D-4)。

四条钉裁定的测试:drop_items() 吐完**不清空**故死后掉双份(D-3,别"顺手修好")、
die() 立即终止任务(D-4)、die() 四件效果齐、die() 打满血 boss hp 归 0。
P4-b 三条:坏类型 no-op+计数、n 先钳后饱和(两步都要,只钳会溢出、只饱和会让负数回绕)、
非敌 owner Fault。

道具类型五常量入 consts ①段。**不入②段**——②段 join 校验是抓"符号 vs 可加载表行"漂移的,
而 item_cfg 是定长数组、类型数编译期冻结,没有可抓的漂移;当前 join 还硬编码校验对象是
appearances,放②会得到一个拿 appearances.len() 校验道具 id 的假检查。②段注释里把道具
类型符号当例子的那句一并改掉(它误导了本刀的第一版设计)。

金向量逐字节不变(纯新增,无脚本调用)。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 文档与收口

**Files:**
- Modify: `docs/ecl-lang.md`（手写语义节）
- Modify: `docs/ecl-ops.md`（syscall 号表追四行）
- Modify: `stg-world-design.md`（敌人字段表 `drop_table` → `drop_count`）
- Modify: `crates/stg-core/src/spell.rs`（订正一段过时注释，见 Step 4）
- Modify: `PROGRESS.md`

- [ ] **Step 1: 确认生成段已是最新**

```bash
cargo run -p stg-harness -- gen-ecl-meta && git diff --stat
```
**预期无改动**（T3 已跑过）。有 diff 就是前面漏了同步，带上并在报告里说明。

- [ ] **Step 2: `docs/ecl-lang.md` 手写语义节**

生成段只有签名，语义必须手写。**三块**：

**(a) 四个内建的语义**
- `drop_clear()` / `drop_add(type, n)`：待掉落计数是**敌身上的可变状态**，`drop_add` 只增不减、
  计数上限 255 饱和、类型用 `ITEM_POWER` 等引擎常量。
- `drop_items()`：立刻撒，**吐完不清空**——所以 `drop_items(); die();` 会掉**两份**。
  这是刻意的（照 ZUN），想只掉一份就别在 `die()` 前调它。**要显眼**，这是最容易踩的一个坑。
- `die()`：就地阵亡，走完整死亡效果（掉落 + 加分 + 死亡事件 + 死亡特效），并**立即终止本任务**
  ——`die()` 后面的语句不执行。

**(b) 三条死亡路径的对照表**（这块最重要，是脚本作者最容易搞混的）

| 路径 | 掉落 | 加分 | 死亡事件/特效 | 触发方式 |
|---|---|---|---|---|
| 被自机打死 | ✔ | ✔ | ✔ | hp ≤ 0 |
| `die()` | ✔ | ✔ | ✔ | 脚本显式 |
| 主任务跑完（D9） | ✘ | ✘ | ✘ | 自然 return |

配一个能编译的例子：想"自然退场也掉落"就在 return 前 `drop_items()`；想"就地阵亡"就 `die()`。

**(c) 两条坑**
- **dying 的敌当帧仍参与碰撞**（spec §6 / D-9）：`die()` 掉的敌人当帧**体碰仍成立、仍能撞死
  自机**，相位 9 才回收。这与被打死同性质，只是窗口更长。
- `die()` / `drop_*` 的 self **必须是敌**（`spawn_enemy` 的 `task` 参那种任务），
  关卡根脚本里调会 Fault。

> ⚠️ `ecl-lang.md` 的围栏示例是**真编译**的（harness 有 `every_ecl_fenced_example_in_doc_compiles`）。
> 写完务必 `cargo test -p stg-harness` 验一下。

- [ ] **Step 3: `docs/ecl-ops.md` syscall 号表追四行**

追到 **syscall 表**（`| 50 | add_score |` 那张，约 110 行起；**不是**上面那张 `| 50 | SPAWN |`
的 op 表，那是另一套编号）。四行 58/59/60/61，照邻行的详略口径写：参数收窄、P4 处置、
owner 类别限制。`die` 那行要写明"表层降低成 SYS + OP_KILL_SELF 两条指令，故本任务立即终止"。

- [ ] **Step 4: 订正 `spell.rs` 的一段过时注释**

`crates/stg-core/src/spell.rs:472-480` 有一大段论证："`hp_break` 三路 OR 里的 `ENEMY_DYING`
那路无法被判别式测试覆盖"，理由是当时唯一置 `ENEMY_DYING` 的路径是 `hp<=0`，与第三路
`hp<=threshold` 恒同真。

**这个论证已经过时**：D9 落地后自燃会在 hp 远高于 threshold 时置 dying，本刀的 `die()`
更是如此。把那段注释订正成事实，并**补一条真正判别那路 OR 的测试**——用 `die()` 打一个
绑卡且 hp 远高于 threshold 的 boss，断言收卡结算发生；删掉 `hp_break` 里 `ENEMY_DYING`
那一路 OR，这条测试必须红（**实测确认，不要只写注释声称**）。

- [ ] **Step 5: `stg-world-design.md` 敌人字段表**

`stg-world-design.md:438` 的挂钩行 `main_task: Handle, death_script: u16, drop_table: u16`
改成 `drop_count`，并一句话说明它是逐类型计数、`drop_table` 退化成生成参数。
**这是权威设计文档**——改动本身过评审（本刀的 spec 已是评审记录），改完在报告里点出改了哪行。

- [ ] **Step 6: `PROGRESS.md`**

- **史加一行**（表格最上方，日期用 `date +%F` 取真实日期）：里程碑名 **敌人死亡效果刀**，
  一句话概括三件 + 关键判据 + 金向量两段漂移的归因。照既有行的密度写。
- **「现在」段重写**（≤10 行，重写不追加）：位置换成本刀；「在飞」= 无；
  **保留 B26 余量那条现状**（`visible_instances` 断言 + 可玩性目验，都要有头环境）。

- [ ] **Step 7: 全绿收口**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- check godot/ecl/demo
cargo run -p stg-harness -- check crates/stg-harness/scenes/rainbow.ecl
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```

**两个冒烟都要真跑**——本刀改了敌人池布局与死亡路径，桥面和真工程都可能受影响
（`godot_smoke.ecl` 与 demo 局都有会死的敌人）。

金向量：本任务纯文档 + 一条测试，**必须逐字节不变**。

- [ ] **Step 8: 提交**

```bash
git commit -m "$(cat <<'EOF'
docs: 敌人死亡效果刀收口——三条死亡路径对照表 + 号表四行 + 订正 spell.rs 过时论证

ecl-lang.md 手写节:四个内建的语义、**三条死亡路径的对照表**(被打死/die()/主任务跑完 D9
——掉落·加分·事件三列的差异,这是脚本作者最容易搞混的一处)、两条坑(drop_items 吐完不清空
故 die() 前调会掉双份;dying 敌当帧仍参与碰撞仍能撞死自机)。

顺带订正 spell.rs 一段过时论证:它称"hp_break 三路 OR 里的 ENEMY_DYING 那路无法判别式覆盖"
(因当时唯一置 dying 的路径是 hp<=0,与第三路恒同真)。D9 落地后已不成立,die() 更是如此
——补一条用 die() 打绑卡满血 boss 的测试真正判别那路,删该路 OR 会红(实测确认)。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## 自审记录

- **spec 覆盖**：§4.1 池字段→T1；§4.2 两函数→T1(`spill_drops`)/T2(`kill_enemy`)；
  §4.3 四 syscall→T3；§4.4 展开助手→T1；§4.5 `die()` 降低→T3；§4.6 道具常量→T3；
  §5 时序→T4 文档；§6 碰撞坑→T4 文档；§7 P4 七条→T1 Step 5/7 + T3 Step 1/3；
  §8 测试十条→T1 Step 1/7、T2 Step 1、T3 Step 1、T4 Step 4（`die()` 打绑卡 boss 那条）；
  §9 破坏面→T3 Step 8（ENGINE_VER）+ T4；§11 顺带发现→T4 Step 4。
- **对 spec 的两处订正**已在开头单列一节（`hp.min(0)`；道具常量放①段），并把订正理由写进
  T2/T3 的代码注释——免得将来有人照 spec 改回去。
- **类型一致性**：`drop_counts` 返回 `([u8; ITEM_TYPE_COUNT], bool)`，T1 Step 4 的生成路径与
  T1 Step 3 的定义一致；`spill_drops(e: usize, ...)` / `kill_enemy(e: usize, ...)` 取池索引，
  handle 版在 T3 的 `world.rs` 里包一层；`add_enemy_drop(h, item_type: i32, n: i32)` 与
  T3 派发臂的 `pop` 产物类型一致。
- **金向量三段记账**：T1 漂（布局）/ T2 漂（加分）/ T3、T4 不漂。每个任务的 Step 都写了
  预期方向——**"该差的差、该同的同"两个方向都要验**，只验一个方向等于没验。
- **本刀的判别力策略**：T1 走**特征化测试**（先钉现状再重构，唯一能证明"RNG 消耗顺序没变"
  的手段）；T2/T3 走标准 TDD（新行为，先红后绿），但四条钉人类裁定的测试要额外当心——
  它们守的是"刻意的怪语义"（吐完不清空 / dying 仍碰撞），最容易被将来的人当 bug 修掉，
  所以注释里都写明了"这是裁定，不是 bug"。
