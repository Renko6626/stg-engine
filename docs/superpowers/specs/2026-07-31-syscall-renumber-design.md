# syscall 号表百分区重排 —— 设计（2026-07-31）

> 2026-07-31 brainstorm 拍板。**冻结面变更**（CLAUDE.md 改动前自检第 3 条：改 op 清单 ⇒ 过评审 + bump `engine_ver`）。
> 对标 `xform.rs` 的既有先例（族号制 v2，2026-07-16）与 ZUN ECL 的百分区。

## 1. 问题：分区不是没有，是**族太小、撑爆了两次**

`syscall.rs` 里有五条族号注释，说明当初就是**十位族号制**（同 `xform.rs`）：

```
// 0x：读——世界/自机/随机/变量
// 2x：写——创建/世界变更
// 3x：写——弹 setter 族（self owner 必须是 BULLET；按 motion.rs 九连顺序编号）
// 4x：读——瞄准
// 5x：写——账面/表现声明族
```

**它已经溢出过两次，而且都没被记下来：**

| 族 | 容量 | 实占 | 后果 |
|---|---|---|---|
| `0x` 读族 | 10 | **13**（0–12） | `SPELL_TIMER`(11)、`ENEMY_HP`(12) 挤进 `1x`，而 `1x` 从未被命名 |
| `6x` shooter | 10 | **15**（62–76） | 直接吃掉 `7x` 整族 |

撑爆之后，77 号往后就**没有族可落**了，于是最近四刀纯自增：

```
77–79  atan2 / dist / nearest_enemy
80–82  enemy_x / enemy_y / enemy_alive     ← 该与 12 号 enemy_hp 同族
83–86  move_vel ×4                          ← 该与 24 号 move_enemy_to 同族
87–90  self_vx / vy / speed / angle         ← 该与 3–5 号 self_x/y/hp 同族
```

**读族现已裂成三段**（3–12、80–82、87–90），敌运动裂成两段（24、83–86）。

> 归因说清楚：这四刀都是最近做的，每一刀单看"接着往下加"都自然。**根因不是纪律松，是十位族号对一张 74 条且在长的表本来就不够用**——`xform` 的 op 表只有 ~20 条，十位够；syscall 表不够。

## 2. 决定：改百分区

**理由三条：**

1. **容量**：`u16` 号空间，百分区每族 100 格。shooter 现有 15 条、读族 12 条——十位注定再撑爆，百位不会。
2. **可对照**：ZUN ECL 本身就是百分区（`3xx`/`4xx` move/`5xx` drops/`6xx` et\*/`8xx` enm\*）。我们的 `4xx`/`5xx`/`6xx` 能与他**同号段对齐**，将来查 Priw8 的表少一层心算。
3. **本仓先例**：`xform.rs` 的族号制 v2 就是 2026-07-16 在**「回放格式出生前的免费窗口」**里重排的。syscall 表当初没跟着做，所以走到今天。

**族内留空隙**，规矩逐字照抄 `xform.rs`：**新号落族内、永不乱序追加**。族满了要扩，走评审开新族，不许溢出到隔壁。

## 3. 代价：现在近乎为零，且单调上涨

- **无持久化镜像**：`.ecl` 是**启动时从源码编译**的（`compile_units` → `EclImage`），世上不存在需要迁移的编译产物。
- **存档/回放**：头部带 `ENGINE_VER` + 镜像 `content_hash`，重排后旧档**拒载**——这正是既有策略，不是新增破坏。且 `ENGINE_VER 10` 尚未发布。
- **调用方全符号引用**：编译器侧一律 `syscall::SYS_*`，`rg` 确认无任何硬编码数字（`ecl-meta.json` 也**不含** syscall 号，只有 name/signature/doc/params ⇒ 生成物零影响）。
- **测试**：按名不按号（Task 4 期已实证 `builtins.rs` 的 `BUILTINS.len() == names.len()` 断言会兜住漏项）。

拖下去的代价单调涨：每刀多几个号落错位置，且总有一天会有人存下镜像。

**顺带消掉一个既有恶心**：`lang/mod.rs:243` 记着 `SYS_CREATE_BULLET == 20 == OP_ADD` 造成的假阳性。op 号是 `u8`、现最大 60（`OP_SPAWN_PATTERN`）；syscall 全部推到 100 以上后，两个号空间**永久错开**，该类混淆消失。

## 4. 新号表（全 74 条，逐条列全）

### 0xx —— `$` 引擎变量（12）

**与 `parse.rs::resolve_engine_var` 白名单一一对应**，这是全表最自解释的一格：族内只放"脚本写 `$name` 就能读到的东西"。

```
000  frame
010  player_x        011  player_y
020  self_x          021  self_y
022  self_vx         023  self_vy         024  self_speed     025  self_angle
030  self_hp         031  self_hp_max     032  self_age
```

### 1xx —— 查询（10）

函数形式的读口 + 纯数学。与 0xx 的分界是**语法形态**（`$name` vs `f(...)`），不是"读/写"。

```
100  enemy_hp        101  enemy_x         102  enemy_y        103  enemy_alive
110  nearest_enemy
120  aim_player_angle
130  spell_timer
140  atan2           141  dist
150  rand_range
```

> ⚠️ `rand_range` 是本族**唯一有副作用者**（推进模拟 PRNG，随快照回滚）。放这里是按脚本视角
> 的"取一个值"归族；号表注释必须写明它**不是**纯读口。

### 2xx —— 造物（4）

```
200  create_bullet   201  create_bullets_batch
210  spawn_enemy
220  drop_item
```

### 3xx —— 弹操作（9）

self owner 必须是 BULLET；族内顺序照 `motion.rs` 的九连。

```
300  set_bullet_speed    301  set_bullet_angle     302  turn_bullet
310  set_bullet_vel      311  set_bullet_ang_vel   312  set_bullet_accel   313  set_bullet_gravity
320  stop_bullet_fx
330  aim_bullet_at_player
```

### 4xx —— 敌运动（5，对齐 ZUN `4xx`）

```
400  move_enemy_to
410  move_vel        411  move_vel_xy
420  move_angle      421  move_speed
```

### 5xx —— 局面·记账·道具（12，对齐 ZUN `5xx` 的 drops）

```
500  add_score
510  add_lives       511  add_bombs       512  add_power
520  drop_clear      521  drop_add        522  drop_items
530  die
540  clear_bullets
550  bgm             551  bg              552  bg_phase
```

### 6xx —— shooter（15，对齐 ZUN `6xx` 的 et\* 弹管理器）

```
600  sh_reset
610  sh_sprite
620  sh_offset       621  sh_offset_abs   622  sh_offset_rad   623  sh_dist
630  sh_angle        631  sh_speed        632  sh_count
640  sh_aim          641  sh_ring
650  sh_xform        651  sh_task         652  sh_req
660  sh_fire
```

### 7xx —— 控制·事件·符卡·globals（7）

```
700  get_var         701  set_var
710  pulse_signal
720  emit_req
730  boss_set
740  spell_begin     741  spell_end
```

**合计 12+10+4+9+5+12+15+7 = 74**，与现表条数一致，**不增不减一条**。本刀**只改号，不改任何语义、参数序、降级口径**。

> **本表已程序化自检**（spec 期，2026-07-31）：解析上面八个代码块，得 74 条、**无重号、无重名**、
> 族内条数与各节标题声明一致；名字集合与 `syscall.rs` 现有 74 个 `SYS_*` 常量**逐字相同**
> （两向差集皆空）。实现期请重跑同样的比对当作起点，别信这段话——它只证明 spec 写对了，
> 不证明代码搬对了。

### 8xx+ 预留

未来新族落 `8xx` 起。已知候选：高阶轨迹（`moveCircle`/`Ellipse`/`Bezier`，follow-up D16）、敌人连续效果、`stg-py` 若需专用口。

## 5. 需要实测的一件事：稀疏 match 的代价

`vm.rs` 的派发是对 `u16` 的 `match`。现表 0–90 稠密，rustc 大概率生成跳转表；重排后号域跨 0–741 且稀疏，可能退化成二分。**syscall 派发在热路径上**（每次 `OP_SYS` 都走）。

**要求**：重排前后各跑一次 `cargo run --release -p stg-harness -- bench`，把 step 曲线对比记进 `docs/bench-baseline.md` 续表。

- 若回归在噪声内 → 照常。
- 若有可测回归 → **不回退分区**（分区的收益是长期可读性），改用 `match` 之外的派发（例如按族号 `n / 100` 先分派再族内偏移查表）。这条**属于实现选择，不动本 spec 的号表**。

## 6. 迁移检查清单

1. `crates/stg-core/src/ecl/syscall.rs`：74 个号常量重排 + 五条旧族注释换成八条新族注释 + `dispatch` 匹配臂顺序跟着族走（可读性）。
2. `crates/stg-ecl-compiler/src/lang/builtins.rs`：**预期零改动**（全符号引用）——但必须 `rg` 确认，并在报告里给出确认输出。
3. `docs/ecl-ops.md`：号表是本文件的主体，**整表重写**并把八个族的边界与"新号落族内、永不乱序追加"的规矩写在表头。
4. `docs/ecl-lang.md`：表层手册**不出现 syscall 号**，预期零改动——确认。
5. `crates/stg-core/src/lib.rs`：`ENGINE_VER` 10 → 11，理由写**号表重排**（既有取值语义全变，比号表新增硬）。
6. `lang/mod.rs:243` 那条 `SYS_CREATE_BULLET == 20 == OP_ADD` 的假阳性注释：核对是否已因号空间错开而失效，失效则改写或删除。
7. 金向量重新生成（号变 ⇒ 镜像字节变 ⇒ 校验和流变）。base 取重排前 commit，用 `git worktree`。
8. `cargo run -p stg-harness -- gen-ecl-meta`：预期**零 diff**（`ecl-meta.json` 不含号）——确认。
9. 两个冒烟 + `verify-tables` + 两条 `check`。

## 7. 测试

**这一刀的性质是"大规模机械重命名"，判别力要求与常规刀不同**：不是"新行为对不对"，而是"**有没有搬错、搬漏、搬重**"。

- **全表唯一性 + 族归属**：一条测试遍历全部 74 个常量，断言 (a) 号**两两不等**；(b) 每个号落在其**声明族的区间内**（`0..=99` / `100..=199` / …）。搬错族立刻红。
- **条数不变**：断言常量总数 == 74。搬漏一条即红（配合 `builtins.rs` 既有的 `BUILTINS.len() == names.len()`）。
- **既有全部行为测试原样通过**：74 条 syscall 的语义测试**一条都不改**（它们按名引用）。这是本刀最强的回归证据——**若有任何一条需要改，说明搬错了语义，停下来查**。
- **`.ecl` e2e 与金向量**：号变但**行为不变** ⇒ 所有 `.ecl` 源码级测试必须原样绿；金向量流会变（镜像字节变），但**两段场景的世界演化必须逐帧等价**。验收判据：重排前后各跑一次 `golden`，**校验和流不同是预期**，但 `.ecl` 行为测试与两个冒烟全绿。

> **不做变异验证**：本刀不引入判别式，变异（改一个号）会被上面的唯一性/族归属测试直接抓住，
> 没有额外信息量。

## 8. 明确的非目标

- **不改任何 syscall 的语义、参数序、参数类型、降级口径**。本刀纯改号。
- **不改 op 号**（`ops.rs` / `xform.rs`）——它们已是族号制 v2，且 `u8` 空间与 syscall 的 `u16` 从此错开。
- **不趁机加号**：新动词/新读口一律另立一刀。本刀 74 进 74 出。
- **不修 follow-up D19**（五处 `easing as u8` / `dur as u16` 裸截断）——那是行为变更，需单独裁定。
