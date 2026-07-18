# M0-17 shottype 表 + WorldTables 骨架 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 火力→弹型接线（`power_tier` 通电成逐档弹型）+ WorldTables 静态数据层骨架（shottype 表 + 道具表 + 角色参数全家入驻）。

**Architecture:** 新模块 `tables.rs`（纯数据 + `TABLES_V0` 静态实例 + `validate()`）；`&'static WorldTables` 以参数穿线（`step` 签名 +1 参，**不进 World**——I7 无引用、静态表不进快照/校验和）；迁移分三刀走"先零行为搬家、后通电"：T1 纯新增 → T2 穿线+道具表改道（金向量逐位不变门）→ T3 角色参数改道（同门）→ T4 解释器通电（行为变化开始）。上游 spec：`docs/superpowers/specs/2026-07-18-m0-17-shottype-worldtables.md`（**先整读**）。

**Tech Stack:** Rust 2024 / stg-core。复用 `polar_to_vec`/`power_tier`/`create_player_shot`。

## Global Constraints

- **零行为搬家门**（T2/T3）：值逐位同源搬家，**金向量流必须与改前逐位相同**（worktree 基线对比；禁 stash）。
- **shooter 十字段冻结**（spec 拍板 4）；伤害上限不做不留字段；homing 只留 flags bit0 注记。
- **shot_timer 语义**：持 SHOT 累进（wrapping）、松手清零；发射判 `shot_timer % interval == delay % interval`；换档/换 focus 瞬时生效。
- **表校验**：`validate()` 钉 interval>0、radius ∈ [0, MAX_ENTITY_RADIUS]、option 号 ≤ 该档子机数、drop_tables 类型合法。
- git：mingw64 真身 `C:\Program Files\Git\mingw64\bin\git.exe`（cmd 包装器近期起进程不稳）。commit 中文 conventional + 尾签 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 名字适配纪律：计划代码按假设名书写，与实况不符以 codebase 为准，适配记入报告。

---

### Task 1: tables.rs 骨架 + TABLES_V0 + validate（纯新增零波及）

**Files:** Create `crates/stg-core/src/tables.rs`；Modify `lib.rs`（`pub mod tables;`）

**Interfaces (Produces):** `WorldTables`/`CharacterCfg`/`ShotTypeCfg`/`Shooter` 结构 + `pub static TABLES_V0: WorldTables` + `pub fn validate(&self) -> bool`。**本任务无消费者**——既有 205+ 测试全绿不变即回归门。

- [ ] Step 0: `git checkout -b m0-17-shottype`
- [ ] Step 1: 写失败测试（tables.rs 内嵌 tests）：`tables_v0_validates`（validate() == true）、`tables_v0_shape`（5 档×2 态槽全非悬垂、v0 各档 focus 两槽指同列表 `std::ptr::eq`、tier 0 弹 1 路 / tier 2 两路 / tier 4 三路+子机 1 个——**内容量拍板：简化版 2-3 档差异**）、`validate_rejects_bad`（构造 interval=0 / radius 超上限 / option 号越界的坏表各断言 false——判别腿）。
- [ ] Step 2: 实现。结构照 spec「WorldTables 形状」节逐字；`Shooter` 十字段
  `{ interval: u16, delay: u16, dx: Fx, dy: Fx, angle: Angle, speed: Fx, damage: u16, radius: Fx, sprite: u16, option: u8, flags: u8 }`
  （flags doc：bit0 = homing 预留，本刀不解析）。`TABLES_V0` 内容：
  - 角色参数**逐字节抄** player.rs 现值（HIGH_SPEED 294_912 / LOW_SPEED 131_072 / INV_SQRT2 46_341 / HIT_RADIUS 163_840 / GRAZE_RADIUS 16px）；
  - 道具三件**逐字节抄** items.rs 现值（ITEM_CFG 五行 / DROP_TABLES 两表 / ITEM_GRAVITY 9_830）——本任务 items.rs **原件不动**（T2 才改道删除）；
  - shottype v0 内容（**发射节奏抄现状**保 T4 前后可比：interval=4 对齐旧 SHOT_CD_FRAMES；damage=1、speed=12px、radius=4px、sprite=0、直上 angle=Angle(49152)）：
    tier 0-1：1 路 `(dx=0)`；tier 2-3：2 路 `(dx=∓8px)`；tier 4：3 路 `(dx=0,∓12px)` + 子机 1 个
    `option_pos[4] = [(-20px, 8px)]` 上的 1 路 shooter（option=1）；各档 focus 两槽指同一列表。
- [ ] Step 3: 全套门（fmt/test/clippy）+ Commit `feat(core): WorldTables 骨架 + TABLES_V0——shottype/道具/角色参数纯数据层（零消费者）`

### Task 2: `&WorldTables` 穿线 + 道具表改道（零行为搬家）

**Files:** Modify `step.rs`（签名）、`world.rs`+`world/{settle,integrate,collide}.rs`（ITEM_CFG/DROP_TABLES/ITEM_GRAVITY 读点改道）、`items.rs`（删三件，`ItemTypeCfg` 结构体定义**迁往 tables.rs**；池/类型常量/哨兵/POWER_MAX 等账本常数留下）、`stg-harness/src/main.rs`、全部测试调用点

**Interfaces:** `pub fn step(world, tables: &WorldTables, input)` / `step_with_director(world, tables, input, director)`；相位函数按需 +参（`update_players(&mut self, tables)` 本任务先不加——T4 才需要；`integrate(tables)`/`settle(tables)` 本任务加）。写 API 中读表者（`spawn_drop`/`drop_item`/`spawn_star_at`/`attract_all_items`/collide 行 5 拾取半径读点）+`tables: &WorldTables` 参（D12 既定签名形态）。

- [ ] Step 1: **先跑改前金向量存基线**（`golden --out g17-pre.txt`）。
- [ ] Step 2: test_support 加糖 `pub(crate) fn step_t(w: &mut World, input: &InputFrame) { crate::step::step(w, &crate::tables::TABLES_V0, input) }`；机械迁移全部调用点（测试用糖；harness 显式传 `&TABLES_V0`）。相位/写 API 签名照上节。`director` 闭包签名不动（导演拿 `&mut WorldBody`，不需要表——需要时另议）。
- [ ] Step 3: 全套门 + **金向量与基线 diff 逐位相同**（零行为门）+ Commit `refactor(core): &WorldTables 穿线 + 道具表迁家——step 签名 +1 参（金向量逐位不变实证）`

### Task 3: 角色参数改道（零行为搬家）

**Files:** Modify `player.rs`（删移速/半径五常量与编译期断言；`spawn` 取值改道）、`world/player.rs`（移动逻辑读表）、`step.rs`（`World::new` 内引 `TABLES_V0` 喂 spawn——**v0 妥协**：避免 `World::new(seed)` 百处调用点改签名；多表时代加 `new_with_tables`，follow-ups 记档）

- [ ] Step 1: `PlayerState::spawn(character_id, cfg: &crate::tables::CharacterCfg)`；`World::new` 内 `TABLES_V0.characters[0]`；`update_players` 移速三值读 `tables.characters[player.character_id]`（T2 未给 update_players 加参——本任务加）。player.rs 编译期断言删除（T1 validate 的 radius 腿接管；补一条单测 `spawn 半径 == TABLES_V0 表值` 逐位）。
- [ ] Step 2: 全套门 + **金向量与 T2 基线逐位相同** + Commit `refactor(core): 角色参数迁表——spawn/移动读 CharacterCfg（金向量逐位不变实证）`

### Task 4: shot_timer + 相位 3 解释器通电（行为变化开始）

**Files:** Modify `player.rs`（`shot_cd: u8` → `shot_timer: u16`）、`world/player.rs`（发弹段整替）、删 `SHOT_SPEED/SHOT_RADIUS/SHOT_CD_FRAMES/SHOT_DAMAGE`

- [ ] Step 1: 失败测试（world/player.rs 或 step.rs tests，从众）：
  - `shottype_tier_bullet_counts`：power=0 持 SHOT 步进 interval 帧 → 恰 1 弹；power=250（tier 2）→ 恰 2 弹且 x 偏移 ∓8px 逐位；power=400 → 3 弹 + 子机弹出生点 = 自机位 + (-20px, 8px) 逐位（**变异 ①③ 的判别腿**）；
  - `shot_timer_phase_and_release_reset`：持 SHOT 记首发帧号；松 1 帧再持，首发延迟与初次一致（**变异 ② 判别腿**）；持续持按出弹间隔恰 = interval；
  - `focus_indexes_focused_set`：BTN_SLOW 持下走 `sets[tier][1]`（v0 两槽同列表——用 `std::ptr::eq` 断言索引到位即可，内容差异后补）。
- [ ] Step 2: 实现解释器（spec「相位 3 解释器」节逐字）：SHOT 持下 `shot_timer = shot_timer.wrapping_add(1)` 松手置 0（**先加后判还是先判后加：发射判定用自增前还是后的值，实现时定一种并在测试里钉死**）；逐 shooter 升序判 `shot_timer % interval == delay % interval` → 出生点合成 → `create_player_shot`。
- [ ] Step 3: 全套门 + 金向量**双跑**一致（流已预期变化，无需与旧基线比）+ Commit `feat(world): shottype 表通电——相位 3 解释器/shot_timer/逐档弹型（judgment 测试群）`

### Task 5: 金向量拔档入流

- [ ] 导演加块（续现有编号）：帧 200 `b.players[0].power = 250;`、帧 400 `= 400;`（诊断场景直写合法——拔 tier 2/tier 4 弹型入流，子机弹参与碰撞/擦弹链）+ 函数 doc 两句；双跑 diff IDENTICAL；帧 200/201 校验和 vs 拔档前状态变化引证（改动本身即证据，不需 worktree）。
- [ ] Commit `feat(harness): 金向量导演拔档——200/400 帧上 tier 2/4 压逐档弹型入流`

### Task 6: 变异 + 收尾 + 终审 + 收枝

- [ ] 变异三杀（反向 Edit 还原，禁 checkout/restore/stash）：① 解释器 tier 恒 0 → 逐档弹数红；② 松手不清零 → 首发延迟红；③ option_pos 偏移不加 → 子机出生点红。树净全绿。
- [ ] 回写：`stg-world-design.md` A3 表清单落地括注（WorldTables v0 骨架 + 全家入驻 + 内容哈希占位）/ D6·A8 自机发弹改"shottype 表驱动（M0-17）"/ D12 签名注（&WorldTables 已穿线）；`docs/follow-ups.md`：homing 单刀（转向率存放两案：全局常量 vs ShotPool 加字段）+ 内容哈希待文件加载刀 + `World::new` 内引 TABLES_V0 的多表后续；`PROGRESS.md` 史行 + 现在段（下一步候选：M1 ECL · bomb · homing · 激光池）。
- [ ] 全绿门 → docs commit → 终审（最强模型全分支）→ finishing（问用户收枝）。

---

## Self-Review 记录

- **Spec 覆盖**：骨架/内容→T1；穿线+道具搬家→T2；角色迁表→T3；计时器+解释器→T4；金向量→T5；变异/回写→T6。零行为门显式落在 T2/T3。
- **占位说明**：T2/T3 是机械迁移，逐调用点清单靠 worker 现场 grep（改动面在 Interfaces 节框死）；T4 自增/判定顺序留一处实现自由度但要求测试钉死所选语义——非占位而是防过度指定。
- **类型一致**：`step_t` 糖（T2 产出、T3/T4 测试消费）；`Shooter` 十字段 T1/T4 一致；`TABLES_V0` 全计划同名。
