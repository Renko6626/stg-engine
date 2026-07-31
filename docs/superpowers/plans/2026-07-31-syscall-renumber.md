# syscall 号表百分区重排 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 74 个 syscall 号从"撑爆的十位族号制"重排成百分区，族内留空隙，**只改号、不改任何语义**。

**Architecture:** 八个百分区族（`0xx` 引擎变量 / `1xx` 查询 / `2xx` 造物 / `3xx` 弹操作 / `4xx` 敌运动 / `5xx` 局面记账 / `6xx` shooter / `7xx` 控制事件）。号表在 `syscall.rs` 一处定义，调用方全部符号引用，故重排面收敛在一个文件 + 一份文档。

**Tech Stack:** Rust 1.94.0（edition 2024）。

**权威设计：** [`docs/superpowers/specs/2026-07-31-syscall-renumber-design.md`](../specs/2026-07-31-syscall-renumber-design.md)。计划与 spec 冲突时**以 spec 为准**，并把冲突报上来。

## Global Constraints

- **本刀纯改号。不改任何 syscall 的语义、参数序、参数类型、降级口径、owner 规则。**
- **74 进 74 出**：不增不减一条。新动词/新读口一律另立一刀。
- **判定这刀是否搬对的最强证据：74 条 syscall 的既有行为测试一条都不用改。**
  若发现**任何**一条需要改，说明搬错了语义 —— **停下来查，不要改测试迁就实现**。
- 族内留空隙的规矩（写进代码注释）：**新号落族内、永不乱序追加**。族满了走评审开新族，不许溢出到隔壁。
- `ENGINE_VER` 10 → 11（**号表重排 = 既有取值语义全变**，比号表新增硬）。
- 金向量**预期会变**（镜像字节变），但 `.ecl` 行为测试与两个冒烟必须全绿。
- 收口全绿：`cargo fmt --all` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace`。
- ⚠️ **禁用 `git checkout <file>` 与 `git stash`**——本仓已被咬过两次，一律用编辑撤回。金向量/bench 取 base 用 `git worktree add`。
- commit 结尾附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

## 冻结的新号表（全 74 条，逐字照搬，勿自行调整）

```
── 0xx  $ 引擎变量（12；与 parse.rs::resolve_engine_var 白名单一一对应）
000  frame
010  player_x        011  player_y
020  self_x          021  self_y
022  self_vx         023  self_vy         024  self_speed     025  self_angle
030  self_hp         031  self_hp_max     032  self_age

── 1xx  查询（10；函数形态的读口 + 纯数学）
100  enemy_hp        101  enemy_x         102  enemy_y        103  enemy_alive
110  nearest_enemy
120  aim_player_angle
130  spell_timer
140  atan2           141  dist
150  rand_range      ← 本族唯一有副作用者（推进模拟 PRNG）

── 2xx  造物（4）
200  create_bullet   201  create_bullets_batch
210  spawn_enemy
220  drop_item

── 3xx  弹操作（9；self owner 必须是 BULLET）
300  set_bullet_speed    301  set_bullet_angle     302  turn_bullet
310  set_bullet_vel      311  set_bullet_ang_vel   312  set_bullet_accel   313  set_bullet_gravity
320  stop_bullet_fx
330  aim_bullet_at_player

── 4xx  敌运动（5；对齐 ZUN 4xx）
400  move_enemy_to
410  move_vel        411  move_vel_xy
420  move_angle      421  move_speed

── 5xx  局面·记账·道具（12；对齐 ZUN 5xx）
500  add_score
510  add_lives       511  add_bombs       512  add_power
520  drop_clear      521  drop_add        522  drop_items
530  die
540  clear_bullets
550  bgm             551  bg              552  bg_phase

── 6xx  shooter（15；对齐 ZUN 6xx 的 et*）
600  sh_reset
610  sh_sprite
620  sh_offset       621  sh_offset_abs   622  sh_offset_rad   623  sh_dist
630  sh_angle        631  sh_speed        632  sh_count
640  sh_aim          641  sh_ring
650  sh_xform        651  sh_task         652  sh_req
660  sh_fire

── 7xx  控制·事件·符卡·globals（7）
700  get_var         701  set_var
710  pulse_signal
720  emit_req
730  boss_set
740  spell_begin     741  spell_end
```

常量名 = `SYS_` + 上表名字的大写（`frame` → `SYS_FRAME`，`sh_offset_abs` → `SYS_SH_OFFSET_ABS`）。
**全部 74 个常量名与现表逐字相同，一个都不改名。**

---

## Task 1：号表重排 + 族结构测试

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（74 个号常量 + 族注释 + `dispatch` 匹配臂顺序）
- Verify-only（预期零改动）: `crates/stg-ecl-compiler/src/lang/builtins.rs`

**Interfaces:**
- Produces：74 个重排后的 `SYS_*` 常量（取值见上表）。**名字与语义全部不变**，故所有下游调用方零改动。

- [ ] **Step 1：先做起点比对（不是测试，是防搬错的地基）**

在动任何代码前，跑这段，把输出贴进报告：

```bash
python3 - <<'PY'
import re
s=open('crates/stg-core/src/ecl/syscall.rs',encoding='utf-8').read()
cur=re.findall(r'pub const (SYS_[A-Z_0-9]+): u16 = (\d+)',s)
print("现表条数:",len(cur))
print("现表名字集合已存盘")
open('/tmp/syscall-names-before.txt','w').write('\n'.join(sorted(n for n,_ in cur)))
PY
```

Expected：`现表条数: 74`。这份名单是 Step 5 的比对基准。

- [ ] **Step 2：写失败测试（族结构）**

放 `crates/stg-core/src/ecl/syscall.rs` 的 `mod tests`：

```rust
/// 【本刀的主判据】号表族结构：74 条、无重号、每条落在其声明族的百位区间内。
///
/// 这一刀是大规模机械重排，判别力要求与常规刀不同——不是"新行为对不对"，而是
/// "**有没有搬错、搬漏、搬重**"。故判据是号表自身的结构性质，不是某条 syscall 的行为。
#[test]
fn syscall_table_is_hundred_partitioned_and_unique() {
    // (号, 名, 期望族号)——逐条照 spec §4 的表；改动本表 = 改冻结面 = 过评审。
    let table: &[(u16, &str, u16)] = &[
        (SYS_FRAME, "frame", 0),
        (SYS_PLAYER_X, "player_x", 0),
        (SYS_PLAYER_Y, "player_y", 0),
        (SYS_SELF_X, "self_x", 0),
        (SYS_SELF_Y, "self_y", 0),
        (SYS_SELF_VX, "self_vx", 0),
        (SYS_SELF_VY, "self_vy", 0),
        (SYS_SELF_SPEED, "self_speed", 0),
        (SYS_SELF_ANGLE, "self_angle", 0),
        (SYS_SELF_HP, "self_hp", 0),
        (SYS_SELF_HP_MAX, "self_hp_max", 0),
        (SYS_SELF_AGE, "self_age", 0),
        (SYS_ENEMY_HP, "enemy_hp", 1),
        (SYS_ENEMY_X, "enemy_x", 1),
        (SYS_ENEMY_Y, "enemy_y", 1),
        (SYS_ENEMY_ALIVE, "enemy_alive", 1),
        (SYS_NEAREST_ENEMY, "nearest_enemy", 1),
        (SYS_AIM_PLAYER_ANGLE, "aim_player_angle", 1),
        (SYS_SPELL_TIMER, "spell_timer", 1),
        (SYS_ATAN2, "atan2", 1),
        (SYS_DIST, "dist", 1),
        (SYS_RAND_RANGE, "rand_range", 1),
        (SYS_CREATE_BULLET, "create_bullet", 2),
        (SYS_CREATE_BULLETS_BATCH, "create_bullets_batch", 2),
        (SYS_SPAWN_ENEMY, "spawn_enemy", 2),
        (SYS_DROP_ITEM, "drop_item", 2),
        (SYS_SET_BULLET_SPEED, "set_bullet_speed", 3),
        (SYS_SET_BULLET_ANGLE, "set_bullet_angle", 3),
        (SYS_TURN_BULLET, "turn_bullet", 3),
        (SYS_SET_BULLET_VEL, "set_bullet_vel", 3),
        (SYS_SET_BULLET_ANG_VEL, "set_bullet_ang_vel", 3),
        (SYS_SET_BULLET_ACCEL, "set_bullet_accel", 3),
        (SYS_SET_BULLET_GRAVITY, "set_bullet_gravity", 3),
        (SYS_STOP_BULLET_FX, "stop_bullet_fx", 3),
        (SYS_AIM_BULLET_AT_PLAYER, "aim_bullet_at_player", 3),
        (SYS_MOVE_ENEMY_TO, "move_enemy_to", 4),
        (SYS_MOVE_VEL, "move_vel", 4),
        (SYS_MOVE_VEL_XY, "move_vel_xy", 4),
        (SYS_MOVE_ANGLE, "move_angle", 4),
        (SYS_MOVE_SPEED, "move_speed", 4),
        (SYS_ADD_SCORE, "add_score", 5),
        (SYS_ADD_LIVES, "add_lives", 5),
        (SYS_ADD_BOMBS, "add_bombs", 5),
        (SYS_ADD_POWER, "add_power", 5),
        (SYS_DROP_CLEAR, "drop_clear", 5),
        (SYS_DROP_ADD, "drop_add", 5),
        (SYS_DROP_ITEMS, "drop_items", 5),
        (SYS_DIE, "die", 5),
        (SYS_CLEAR_BULLETS, "clear_bullets", 5),
        (SYS_BGM, "bgm", 5),
        (SYS_BG, "bg", 5),
        (SYS_BG_PHASE, "bg_phase", 5),
        (SYS_SH_RESET, "sh_reset", 6),
        (SYS_SH_SPRITE, "sh_sprite", 6),
        (SYS_SH_OFFSET, "sh_offset", 6),
        (SYS_SH_OFFSET_ABS, "sh_offset_abs", 6),
        (SYS_SH_OFFSET_RAD, "sh_offset_rad", 6),
        (SYS_SH_DIST, "sh_dist", 6),
        (SYS_SH_ANGLE, "sh_angle", 6),
        (SYS_SH_SPEED, "sh_speed", 6),
        (SYS_SH_COUNT, "sh_count", 6),
        (SYS_SH_AIM, "sh_aim", 6),
        (SYS_SH_RING, "sh_ring", 6),
        (SYS_SH_XFORM, "sh_xform", 6),
        (SYS_SH_TASK, "sh_task", 6),
        (SYS_SH_REQ, "sh_req", 6),
        (SYS_SH_FIRE, "sh_fire", 6),
        (SYS_GET_VAR, "get_var", 7),
        (SYS_SET_VAR, "set_var", 7),
        (SYS_PULSE_SIGNAL, "pulse_signal", 7),
        (SYS_EMIT_REQ, "emit_req", 7),
        (SYS_BOSS_SET, "boss_set", 7),
        (SYS_SPELL_BEGIN, "spell_begin", 7),
        (SYS_SPELL_END, "spell_end", 7),
    ];

    assert_eq!(table.len(), 74, "74 进 74 出：本刀不增不减一条");

    // (a) 族归属：搬错族立刻红
    for &(num, name, fam) in table {
        assert_eq!(
            num / 100,
            fam,
            "{name}(={num}) 应落在 {fam}xx 族，实得 {}xx",
            num / 100
        );
    }

    // (b) 两两不等：搬重立刻红（O(n²) 但 n=74，测试里无所谓）
    for (i, &(a, na, _)) in table.iter().enumerate() {
        for &(b, nb, _) in &table[i + 1..] {
            assert_ne!(a, b, "{na} 与 {nb} 撞号（都是 {a}）");
        }
    }

    // (c) 与 op 号空间错开：op 是 u8、现最大 60（OP_SPAWN_PATTERN），
    //     syscall 全部 >= 100 后两个号空间永久不相交（消掉 lang/mod.rs 那条假阳性）。
    for &(num, name, _) in table {
        assert!(
            num >= 100 || num < 100,
            "占位断言，见下方 0xx 族说明"
        );
        let _ = name;
    }
}
```

> ⚠️ **上面 (c) 那段是故意写坏的占位**——`num >= 100 || num < 100` 恒真，是**空断言**。
> `0xx` 族的号本来就 < 100，所以"全部 ≥ 100"这个说法对 `0xx` 不成立，spec §3 那句话
> 需要收窄。**请你自己判断并改写 (c)**：要么删掉它（op 错开这件事由 `1xx..7xx` 天然满足、
> `0xx` 与 op 的重叠是既有状态、本刀不承诺解决），要么把断言收窄成对非 `0xx` 族成立。
> **不要留一个恒真的断言在测试里。** 你怎么处理请写进报告。

- [ ] **Step 3：跑测试确认失败**

```bash
cargo test -p stg-core syscall_table_is_hundred_partitioned_and_unique
```
Expected：FAIL —— 族归属断言先红（如 `enemy_hp(=12) 应落在 1xx 族，实得 0xx`）。

- [ ] **Step 4：重排号常量 + 族注释**

按上表改 74 个 `pub const` 的取值。**同时**把文件里现有的五条旧族注释

```
// 0x：读——世界/自机/随机/变量
// 2x：写——创建/世界变更
// 3x：写——弹 setter 族（self owner 必须是 BULLET；按 motion.rs 九连顺序编号）
// 4x：读——瞄准
// 5x：写——账面/表现声明族（整局流程刀 spec §4；owner 类别无限制，STAGE 任务常发）。
```

换成八条新族注释，并在文件头（号常量区之前）加一段总纲：

```rust
// ── syscall 号表（冻结；百分区制，2026-07-31 重排拍板）─────────────────────────
// 百位 = 族号：0xx `$` 引擎变量 / 1xx 查询 / 2xx 造物 / 3xx 弹操作 / 4xx 敌运动 /
// 5xx 局面·记账·道具 / 6xx shooter / 7xx 控制·事件·符卡 / 8xx+ 预留。
//
// **族内留空隙：新号落族内、永不乱序追加。** 族满了走评审开新族，不许溢出到隔壁——
// 上一版是十位族号制，`0x` 读族（容量 10）实占 13、`6x` shooter（容量 10）实占 15，
// 两次静默溢出之后 77 号起就没族可落了，最近四刀只好纯自增，读族因此裂成三段。
// 那次的教训不是"纪律松"，是**族容量对一张还在长的表本来就不够**。
//
// 4xx/5xx/6xx 与 ZUN ECL 的同号段**有意对齐**（他的 4xx = move、5xx = drops、
// 6xx = et* 弹管理器），方便对着 Priw8 的指令表读。
// 改号 = 冻结面变更 = 过评审 + bump ENGINE_VER（见 spec 2026-07-31）。
```

各族原有的逐条 doc 注释**一字不动**（它们描述语义，本刀不碰语义）。

- [ ] **Step 5：三项机械核对**

```bash
# (1) 名字集合未变（新增/漏搬/改名都会在这里露出来）
python3 - <<'PY'
import re
s=open('crates/stg-core/src/ecl/syscall.rs',encoding='utf-8').read()
now=sorted(re.findall(r'pub const (SYS_[A-Z_0-9]+): u16',s))
before=open('/tmp/syscall-names-before.txt').read().split()
print("条数:",len(now))
print("新增:",sorted(set(now)-set(before)) or "无")
print("丢失:",sorted(set(before)-set(now)) or "无")
PY

# (2) builtins.rs 预期零改动（全符号引用）
git diff --stat crates/stg-ecl-compiler/src/lang/builtins.rs

# (3) 全仓无硬编码 syscall 数字
rg -n 'OP_SYS\s+\d|syscall:\s*\d' crates/ || echo "无硬编码"
```

Expected：条数 74、新增/丢失皆无；`builtins.rs` 零 diff；无硬编码。三项输出都要贴进报告。

- [ ] **Step 6：把 `dispatch` 匹配臂按族重排**

`dispatch` 的 `match` 臂顺序目前跟旧号走。按新族顺序重排（`0xx` → `7xx`，族内按号升序），
每族前加一行 `// ── Nxx 族名 ──` 分隔。**只动顺序与注释，不动任何臂的内容。**

- [ ] **Step 7：跑全量测试**

```bash
cargo test --workspace
```

Expected：**全绿，且一条既有行为测试都没改过。**

⚠️ **若有任何一条 syscall 的行为测试转红 —— 停下来，那说明搬错了语义，不是测试的问题。**
把红的那条与你对它的分析写进报告，状态报 `BLOCKED`，不要改测试迁就实现。

- [ ] **Step 8：commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A && git commit -F- <<'MSG'
refactor(ecl)!: syscall 号表改百分区（74 进 74 出，只改号不改语义）

上一版是十位族号制,0x 读族(容量 10)实占 13、6x shooter(容量 10)实占 15,两次静默
溢出之后 77 号起没族可落,最近四刀纯自增,读族裂成三段(3-12/80-82/87-90)。根因是
族容量对一张还在长的表不够,不是纪律松——xform 只有 ~20 条 op,十位够用。

八族:0xx $ 引擎变量 / 1xx 查询 / 2xx 造物 / 3xx 弹操作 / 4xx 敌运动 /
5xx 局面记账 / 6xx shooter / 7xx 控制事件。4xx/5xx/6xx 与 ZUN 同号段有意对齐。
族内留空隙,新号落族内、永不乱序追加。

判据是号表自身的结构性质(74 条/无重号/族归属),不是某条 syscall 的行为——
**74 条既有行为测试一条未改**,这是搬对了的最强证据。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## Task 2：文档 + `ENGINE_VER` + 金向量/bench 对拍 + 收口

**Files:**
- Modify: `docs/ecl-ops.md`（号表整表重写）、`crates/stg-core/src/lib.rs`（`ENGINE_VER`）
- Modify: `docs/bench-baseline.md`、`PROGRESS.md`
- Check-only: `docs/ecl-lang.md`、`crates/stg-ecl-compiler/src/lang/mod.rs:243`

**Interfaces:**
- Consumes：Task 1 的新号表。

- [ ] **Step 1：`ENGINE_VER` 10 → 11**

`crates/stg-core/src/lib.rs`，照既有体例加理由行：

```
/// 11（2026-07-31，syscall 号表百分区重排）：号表**取值语义全变**——同一个号在新旧两版
///    指向不同的 syscall，旧镜像/旧回放按新表解读会静默走出另一条世界线，必须拒载。
///    这比号表新增（只是多几个号）硬得多。条数与语义均未变，74 进 74 出。
```

- [ ] **Step 2：`docs/ecl-ops.md` 号表整表重写**

现文件的号表主体按旧号排列，**整表重写**：按八族分节，每节一个表格（号 / 名 / 参数 / 一句话），
族标题下写明该族的口径（如 `3xx` 的 "self owner 必须是 BULLET"）。

表头加一段规矩：

```markdown
> **百分区制（2026-07-31 重排）**：百位 = 族号。**新号落族内、永不乱序追加**；族满了走评审
> 开新族，不许溢出到隔壁。`4xx`/`5xx`/`6xx` 与 ZUN ECL 同号段有意对齐（他的 4xx = move、
> 5xx = drops、6xx = et\* 弹管理器）。改号 = 冻结面变更 = 过评审 + bump `ENGINE_VER`。
```

⚠️ **逐条核对语义描述没被改动**——本刀只改号，每条的参数/降级/owner 口径必须与重写前**逐字一致**。

- [ ] **Step 3：两处检查（预期零改动，但必须确认并报告）**

```bash
# (1) 表层手册不出现 syscall 号
rg -n 'syscall\s*\d|号\s*\d{2,3}' docs/ecl-lang.md | head

# (2) gen-ecl-meta 零 diff（ecl-meta.json 不含号）
cargo run -p stg-harness -- gen-ecl-meta && git diff --stat
```

- [ ] **Step 4：`lang/mod.rs:243` 的假阳性注释**

该处记着 `SYS_CREATE_BULLET == 20 == OP_ADD` 造成的假阳性。重排后 `create_bullet` 是 200 号。
**读懂那段注释在说什么**，判断它现在是否还成立：

- 若因号空间错开而失效 → 改写或删除，并在 commit 里说明。
- 若仍以别的形式成立（例如 `0xx` 族的号仍与 op 号重叠）→ **保留并把注释更新成新的例子**。

**不要不看就删。** 你的判断与依据写进报告。

- [ ] **Step 5：bench 对拍（spec §5 要求）**

`dispatch` 的 `match` 从稠密（0–90）变稀疏（0–741），rustc 可能从跳转表退化成二分，
而 syscall 派发**在热路径上**（每次 `OP_SYS` 都走）。

```bash
git worktree add /tmp/wt-bench <本刀第一个 commit 的父>
cd /tmp/wt-bench && cargo run --release -p stg-harness -- bench > /tmp/bench-before.txt
cd /data/sunyunbo/www/stg-engine && cargo run --release -p stg-harness -- bench > /tmp/bench-after.txt
diff -u /tmp/bench-before.txt /tmp/bench-after.txt
git worktree remove /tmp/wt-bench
```

把对比写进 `docs/bench-baseline.md` 续表。

- **回归在噪声内** → 照常，续表记一行"重排无可测影响"。
- **有可测回归** → **不回退分区**（分区的收益是长期可读性）。改用按族分派
  （`n / 100` 先选族、族内偏移查表）。这是**实现选择，不动号表**。若你选择这条路，
  在报告里说明并把改动一并做了。

- [ ] **Step 6：金向量重新生成**

⚠️ **预期会变**（号变 ⇒ 镜像字节变 ⇒ 校验和流变）。

```bash
git worktree add /tmp/wt-golden <本刀第一个 commit 的父>
cd /tmp/wt-golden && cargo run -p stg-harness -- golden --out /tmp/golden-before.txt
cd /data/sunyunbo/www/stg-engine && cargo run -p stg-harness -- golden --out /tmp/golden-after.txt
diff -u /tmp/golden-before.txt /tmp/golden-after.txt | head -20
git worktree remove /tmp/wt-golden
```

**验收判据不是"无差异"，而是**：
1. 差异从帧 0 起连续（镜像字节变 ⇒ 世界初态即变）；
2. **`.ecl` 行为测试与两个冒烟全绿**——这才是"行为没变"的证据，校验和流本身证明不了。

- [ ] **Step 7：`PROGRESS.md` + 全量收口**

`PROGRESS.md`：史加一行（`date +%F`）+「现在」段重写（≤10 行，**保留 B26 余量那条**）。

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- check godot/ecl/demo
cargo run -p stg-harness -- check crates/stg-harness/scenes/rainbow.ecl
cargo test -p stg-harness
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```

⚠️ 两个冒烟都要跑（`ENGINE_VER` 变了）。

- [ ] **Step 8：commit**

```bash
git add -A && git commit -F- <<'MSG'
docs(ecl): 号表百分区收口——ecl-ops 整表重写 + ENGINE_VER 10→11 + bench/金向量对拍

ENGINE_VER 的理由是**号表取值语义全变**(同一个号在新旧两版指向不同 syscall,
旧镜像按新表解读会静默走出另一条世界线,必须拒载),不是条数变——74 进 74 出。

金向量预期变化并已确认形态;行为不变的证据是 .ecl 行为测试与两个冒烟全绿,
校验和流本身证明不了这件事。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
```

---

## 计划自查

**1. Spec 覆盖**

| spec 节 | 落在 |
|---|---|
| §1 问题诊断（十位族撑爆两次） | T1 Step 4 的族注释总纲写进代码 |
| §2 决定（百分区三条理由） | T1 Step 4 注释 + T2 Step 2 文档表头 |
| §3 代价（无持久化镜像/符号引用/生成物零影响） | T1 Step 5 三项核对 + T2 Step 3 两处检查 |
| §4 新号表（74 条） | T1 Step 4，取值在本计划「冻结的新号表」逐字给全 |
| §5 稀疏 match 实测 | T2 Step 5（含"有回归也不回退分区"的处置） |
| §6 迁移检查清单 9 项 | T1 Step 5/6 + T2 Step 1–7 逐项 |
| §7 测试策略 | T1 Step 2（结构判据）+ Step 7（既有行为测试一条不改）+ T2 Step 6 |
| §8 非目标 | Global Constraints 首三条 |

无缺口。

**2. 占位符扫描**：T1 Step 2 的 (c) 段是**故意留的坏断言**（恒真），并已显式标注要求实现者
自己判断并改写、把处置写进报告。这不是遗漏——spec §3 那句"syscall 全部推到 100 以上"
对 `0xx` 族并不成立，我在写测试时才发现，与其在计划里替它糊过去，不如把这个不一致
摆到实现者面前。其余步骤均为可直接落地的完整内容。

**3. 类型一致性**：74 个常量名在本计划的号表与 T1 Step 2 的测试表两处出现，已程序化核对
与 `syscall.rs` 现有常量名逐字相同（spec 期做过，实现期 T1 Step 1/5 再做一次）。
族号 `0..=7` 在两处的归属一致。
