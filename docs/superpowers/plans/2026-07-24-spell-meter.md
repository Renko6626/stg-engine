# 符卡计器机构实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 符卡记账收归引擎机构(`SpellSlot`:计时/bonus 衰减/资格作废/破卡血线自动检测/
伤害下钳/结算入分/事件/req/自动清弹)+ 模式随卡生死(`spell_bound`)+ 三 syscall + `wait_spell`
糖 + rainbow 狗粮化。

**Architecture:** 权威 spec = `docs/superpowers/specs/2026-07-24-spell-meter-design.md`
(含 A2 范围修订评审记录、结束矩阵、模式绑定语义——动手前先读)。三刀:① 世界侧机构
(spell.rs + settle 符卡趟 + 伤害下钳 + boss_ui 自动喂,经 World API 直测)→ ② syscall 层
(三号 + Task.spell_bound + 调度门禁杀 + spawn 继承)→ ③ 表层(builtins + wait_spell 糖 +
rainbow 狗粮化 + 文档)。

**Tech Stack:** Rust 1.92;零新依赖。

## Global Constraints

- `stg-core` 断层线纪律(禁浮点/时钟/宿主 RNG/无序容器);零新增外部依赖。
- **金向量本刀预期双变**:新字段簇 → 校验和取值平移;rainbow 狗粮化(刀 3)→ 演化本身变。
  故 **byte-diff 不是回归闸**;回归判据 = 判别式单测全家 + 跨平台 CI + storm 闸。刀 1/2 不动
  rainbow(golden 只受字段簇平移影响),刀 3 才改脚本。
- **新字段四件套**(收口刀哨兵会逼):checksum(derive 自动)/ copy_into(手写清单补)/
  SaveBytes(derive 自动)/ D10 预算 + 尺寸哨兵数字更新 + 哨兵清单④已列 SaveBytes。
- P4 三铁律:资源耗尽→计数不 panic;调用方违约→no-op+`contract_viol`+`last_status`;
  引擎 bug→debug 断言。整数衰减(I1)、帧计时(I6)、按槽/池索引升序(I4)、无回调(P5)。
- 每任务收尾:`cargo test --workspace` + `cargo fmt --all -- --check` +
  `cargo clippy --workspace --all-targets -- -D warnings`。
- commit 尾:`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`;
  `.superpowers/` 不入库(`git add -A ':!.superpowers'`)。注释中文随邻居。
- 契约值:`SYS_SPELL_TIMER=11 / SYS_SPELL_BEGIN=28 / SYS_SPELL_END=29`;
  `EVT_SPELL_DECLARED=6 / CAPTURED=7 / FAILED=8`;`REQ_SPELL_DECLARE=2 / REQ_SPELL_RESULT=3`;
  `SPELL_SURVIVAL=1<<0 / SPELL_NO_CLEAR=1<<1`;bonus_floor=bonus0/10;reason 1=资格失 2=超时。

---

### Task 1: 世界侧机构（spell.rs + settle 符卡趟 + 伤害下钳 + boss_ui 自动喂）

**Files:**
- Create: `crates/stg-core/src/spell.rs`(SpellSlot + 常量 + settle_spells 逻辑 + tests)
- Modify: `crates/stg-core/src/lib.rs`(`pub mod spell;`,字母序 shots 之后 step 之前)
- Modify: `crates/stg-core/src/world.rs`(WorldBody 加 `spells` 字段 + 内部结算/推进 API)
- Modify: `crates/stg-core/src/world/settle.rs`(趟二尾加伤害下钳 + 新增符卡趟调用)
- Modify: `crates/stg-core/src/world/view.rs`(`WorldView::spells` 访问器)
- Modify: `crates/stg-core/src/step.rs`(copy_into 加 `spells` + 尺寸哨兵数字)
- Modify: `crates/stg-core/src/events.rs`(三事件常量)
- Modify: `crates/stg-core/src/consts.rs`(两 REQ id structural 注册)
- Modify: `crates/stg-core/src/reqs.rs`(约定表两行)

**Interfaces:**
- Consumes: `enemy::ENEMY_DYING`、`player::LIFE_ALIVE`、`create_field`/`FieldInit`/
  `FIELD_RADIUS_FULLSCREEN`/`FIELD_CLEAR_BULLETS`、`boss::{BossUiSlot, MAX_BOSSES}`、
  `EnemyHandle`、`push_event`、`emit_req`、`math::Fx`。
- Produces（Task 2 依赖，均 `pub(crate)`）:
  - `spell::SpellSlot`（pub 字段）+ `SPELL_SURVIVAL`/`SPELL_NO_CLEAR`
  - `WorldBody::spell_begin_internal(slot: usize, boss: EnemyHandle, spell_id: u16, time_limit: u16, bonus0: u32, flags: u8, hp_threshold: i32) -> bool`（成功真；P4-b 判据在内，返回是否 active 化——syscall 据此决定是否 spawn 模式）
  - `WorldBody::spell_end_by_owner(boss: EnemyHandle)`（逃生舱口结算）
  - `WorldBody::spell_frames_left_of(boss: EnemyHandle) -> i32`（-1 = 无绑定）
  - `WorldBody::settle_spells(&mut self, tables)`（符卡趟；step 组装层在 settle 内调）
  - `WorldView::spells(self) -> &'w [SpellSlot]`

- [ ] **Step 1: 金向量基线**

```bash
mkdir -p .superpowers && cargo run -q -p stg-harness -- golden --out .superpowers/golden-pre-spell.txt && wc -l .superpowers/golden-pre-spell.txt
```

- [ ] **Step 2: spell.rs（结构 + 常量 + 纯推进函数）**

```rust
//! 符卡计器机构（spec 2026-07-24）：记账归引擎、控制归脚本。逐帧推进挂 settle 符卡趟。
//! `SpellSlot` POD 入 WorldBody（checksum/copy_into/SaveBytes 三件套）。

use crate::math::Fx;

#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct SpellSlot {
    pub active: u8,
    pub flags: u8,       // bit0 SPELL_SURVIVAL / bit1 SPELL_NO_CLEAR
    pub capture_ok: u8,
    pub _pad: u8,
    pub spell_id: u16,
    pub boss_index: u16,
    pub boss_gen: u16,
    pub frames_left: u16,
    pub hp_threshold: i32,
    pub hp_start: i32,
    pub bonus_now: u32,
    pub bonus_floor: u32,
    pub dec_per_frame: u32,
}

pub const SPELL_SURVIVAL: u8 = 1 << 0;
pub const SPELL_NO_CLEAR: u8 = 1 << 1;

/// 结束原因（EVT_SPELL_FAILED.data[1]）。
pub(crate) const SPELL_FAIL_CAPTURE_LOST: i32 = 1;
pub(crate) const SPELL_FAIL_TIMEOUT: i32 = 2;
```

（`_pad` 显式占位入校验和，零合法——同 events.rs 的 padding 处置。）

- [ ] **Step 3: 写失败测试（spell.rs tests + settle.rs tests）**

`spell.rs` tests：建 World（用 `crate::step::World::new`）+ `create_enemy` 造 boss，经
Task 1 的 World API 直驱（不经 syscall——那是 Task 2）。至少覆盖 spec §9 的 1/2/3/4/6/7 判别式：

```rust
#[cfg(test)]
mod tests {
    use crate::math::Fx;
    use crate::spell::*;

    // 助手：造带 boss 的世界 + 直接 spell_begin_internal（细节照 world.rs settle 邻测样板）
    // 断言 bonus 衰减精确值、地板钳制、资格三触发、结束矩阵各格 score 增量+事件+req、
    // 逐卡血条 ratio、伤害下钳（hp=1000 threshold=300 打 5000 → hp==300）。
    // —— 具体拼装以 world.rs/settle.rs 既有测试助手为样板，断言值是规格：

    #[test]
    fn bonus_decays_linearly_to_floor() {
        // begin(limit=100, bonus0=1000) → floor=100, dec=(1000-100)/100=9
        // 推进 10 帧 → bonus_now == 1000 - 90 == 910；推进 200 帧 → == floor 100 不再降
    }

    #[test]
    fn damage_clamps_at_threshold_only_under_spell() {
        // hp=1000, 卡 threshold=300, 一发 5000 伤害 → hp == 300（非死非负）
        // 对照：同敌无 active 卡，同一发 → hp <= 0 dying（证下钳仅在符卡条件生效）
    }

    #[test]
    fn hp_breakpoint_auto_captures_and_pays_bonus() {
        // 打到 hp<=threshold → CAPTURED：score += bonus_now，EVT_SPELL_CAPTURED，槽清空
    }

    #[test]
    fn miss_voids_capture_then_hp_break_fails() {
        // 资格清 0 后到线 → EVT_SPELL_FAILED reason=资格失，score 不增
    }

    #[test]
    fn timeout_normal_fails_survival_captures() {
        // frames_left 归零：普通卡 FAILED(超时)；SPELL_SURVIVAL 卡 CAPTURED
    }

    #[test]
    fn per_card_hp_ratio_full_at_start() {
        // hp_start=1000 threshold=600：hp=1000→ratio=1.0；hp=600→ratio=0.0
        // boss_ui.hp_ratio 逐帧命中（Fx 真除）
    }
}
```

- [ ] **Step 4: 跑测确认红**（`cargo test -p stg-core spell 2>&1 | tail` → 编译失败，API 未实现）

- [ ] **Step 5: 实现 World API + settle 接线**

**(a)** `world.rs` WorldBody 字段（events_len 附近，生而 `pub(crate)`）:
```rust
/// 符卡计器槽（每 boss 一个；spec 2026-07-24）。生而封口，读经 view().spells()。
pub(crate) spells: [crate::spell::SpellSlot; crate::boss::MAX_BOSSES],
```

**(b)** `world.rs` impl WorldBody（boss_set 附近）加四个 API：
- `spell_begin_internal(...)`: 校验 slot < MAX_BOSSES && time_limit>0 && bonus0 可容
  && hp_threshold <= boss 当前 hp && 该槽未 active && boss 存活；任一不满足 → contract_viol
  + STATUS_BAD_ARGS + 返回 false。成功：写满 SpellSlot 全字段（复用槽写满纪律）——
  `hp_start = enemies.hp[boss_i]`、`bonus_floor = bonus0/10`、
  `dec_per_frame = (bonus0 - floor) / time_limit as u32`、`capture_ok=1`、`active=1`；
  `push_event(EVT_SPELL_DECLARED, data=[spell_id, bonus0])`;`emit_req(REQ_SPELL_DECLARE,
  [spell_id, bonus0, time_limit, survival_bit, 0, 0])`;返回 true。
- `spell_end_by_owner(boss)`: 找 boss 绑定的 active 槽 → 走 HP 路径结算（内部 helper
  `settle_one_spell(slot, captured_path=true)`）；无绑定 no-op（不计 contract——逃生舱口
  重复调用安全，同 spell_end 语义）。
- `spell_frames_left_of(boss) -> i32`: 找绑定 active 槽返回 `frames_left as i32`，无 → -1。
- 私有 `settle_one_spell(&mut self, slot, captured)`：原子结算——`if captured { players[0]
  .score += bonus_now }`;`push_event`（CAPTURED 或 FAILED+reason）；`emit_req(REQ_SPELL_RESULT,
  [spell_id, captured as, 实付, reason, 0,0])`;除非 SPELL_NO_CLEAR，`create_field(全屏消弹
  field: radius=FIELD_RADIUS_FULLSCREEN, flags=FIELD_CLEAR_BULLETS, life=1, ...)`;
  槽全字段清零。

**(c)** `settle_spells(&mut self, tables)` （spec §3 五步，按槽升序）:
资格作废（玩家0 非 ALIVE 或 bomb_phase!=0 → capture_ok=0）→ bonus 衰减 saturating →
破卡检测（句柄失效/ENEMY_DYING/hp<=threshold → settle_one_spell(captured=capture_ok!=0)）→
超时（frames_left==0 → survival?captured:failed；否则 -=1）→ boss_ui 自动喂
（enemy/spell_id/timer/active/hp_ratio=(hp-thr)/(start-thr) 钳[0,1] 真除；phase_left 不动）。

**(d)** `world/settle.rs`:
- 趟二伤害循环**之后**加下钳 pass：遍历 active spells，对绑定敌 `hp_threshold>0` 者
  `enemies.hp[e] = enemies.hp[e].max(threshold)`（防打穿；须在 dying 标记逻辑…注意顺序：
  damage_enemy 里 hp<=0 即标 ENEMY_DYING——所以下钳必须在 damage_enemy 之后、且下钳后
  若 hp>0 则**撤销**误标的 dying？**否**：下钳 pass 读 spells 前置，改为在 damage_enemy
  内部对"绑卡且 threshold>0"的敌先钳后判 dying——见实现注）。**实现定式**：在
  `damage_enemy`（settle.rs:28）里，扣血后、判 `hp<=0` 前，插入"若该敌绑着 active 符卡槽
  且 threshold>0 则 `hp = hp.max(threshold)`"——一处改，天然不误标 dying。
- settle 尾（趟三之后）调 `self.settle_spells(tables)`。

**(e)** `world/view.rs` + `step.rs` + `events.rs` + `consts.rs` + `reqs.rs`:
- view.rs: `pub fn spells(self) -> &'w [crate::spell::SpellSlot] { &self.body.spells }`
- step.rs copy_into: `d.spells = s.spells;`（[T;N] Copy）+ 尺寸哨兵两 EXPECTED 更新（跑测取真值）
- events.rs: `EVT_SPELL_DECLARED=6 / EVT_SPELL_CAPTURED=7 / EVT_SPELL_FAILED=8` + 文档
- consts.rs structural: `REQ_SPELL_DECLARE: u16 as int = 2; REQ_SPELL_RESULT: u16 as int = 3;`
- reqs.rs 模块文档 args 约定表加两行

- [ ] **Step 6: 跑测确认绿 + 金向量（仅取值平移）+ 全绿**

```bash
cargo test -p stg-core spell && cargo test --workspace
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t1.txt
diff .superpowers/golden-pre-spell.txt .superpowers/golden-post-t1.txt || echo "预期不同（字段簇平移；rainbow 未改，仅平移非行为变）"
```

（刀 1 不改 rainbow：golden 仅因 spells 字段簇入校验和而取值平移，行数不变、无 fault。
若行数变或退出非零 = 真回归，停。）

- [ ] **Step 7: Commit**

```bash
git add -A ':!.superpowers'
git commit -m "feat(spell): 符卡计器世界侧机构——SpellSlot + settle 符卡趟 + 破卡血线 + 伤害下钳 + boss_ui 自动喂（刀 1/3）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: syscall 层（三号 + Task.spell_bound + 调度门禁杀 + spawn 继承）

**Files:**
- Modify: `crates/stg-core/src/ecl/task.rs`（Task 加 `spell_bound: u8` 字段）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（三 syscall 常量 + dispatch + sys_* + tests）
- Modify: `crates/stg-core/src/ecl/vm.rs`（run_tasks 门禁加 spell_bound 死亡检查 + OP_SPAWN 继承）

**Interfaces:**
- Consumes: Task 1 的 `WorldBody::{spell_begin_internal, spell_end_by_owner,
  spell_frames_left_of}`;既有 `self_enemy_handle`、`ctx.tasks.spawn`、`push`/`pop`、
  `EclOwner`/OWNER_ENEMY、fire 的 task-spawn 样板（syscall.rs:~405）、OP_SPAWN 继承点
  （vm.rs:~345）、run_tasks owner 门禁（vm.rs:~429-450）。
- Produces（Task 3 依赖）:`SYS_SPELL_BEGIN/END/TIMER` 常量。

- [ ] **Step 1: 写失败测试**

syscall.rs tests（`call` 助手 + `fresh` 样板）:
- `spell_begin` 声明序压栈 `[slot, spell_id, pattern_ref, time_limit, bonus0, flags,
  threshold]`（pattern_ref 用一个已注册 async sub 号；none=负值）→ owner=ENEMY 成功：槽
  active + 模式任务已 spawn（`tasks.iter_alive` 含新任务、其 `spell_bound==slot+1`）;
  owner=STAGE → Fault;槽越界/threshold>hp/重复 → contract_viol no-op;pattern=none → 不 spawn。
- `spell_end`：绑定 boss owner 调 → 槽结算清空;无绑定 → no-op。
- `spell_timer`：绑定 → 压 frames_left;无绑定 → 压 -1。
- vm.rs test：spawn 无限 loop 模式绑卡 → 槽结算后下一轮 run_tasks 该任务被杀（`!is_alive`）;
  其 `spawn` 子任务同死;`fire` 挂弹任务（spell_bound=0）不死。

- [ ] **Step 2: 跑测确认红**

- [ ] **Step 3: 实现**

**(a)** `task.rs`:Task 加字段（`parent` 后、`sp` 前，避免动栈数组对齐）:
```rust
/// 绑定符卡槽号+1（0=不绑）：spell_begin spawn 的模式任务随卡生死（spec §2.1）；
/// spawn 派生子任务继承本值，fire 挂弹任务不继承。POD，checksum/save derive 自动盖。
pub spell_bound: u8,
```
Default 全零已含（`#[derive]` 的 Default 对新 u8 字段自动 0）——确认 Task 是 derive Default
还是手写；task.rs 是**手写 Default**（见文件），故手写块加 `spell_bound: 0,`。

**(b)** `syscall.rs`:
- 常量（读族 `SYS_SELF_HP_MAX=10` 后加 `SYS_SPELL_TIMER=11`;2x 写族 `SYS_EMIT_REQ=27`
  后加 `SYS_SPELL_BEGIN=28`/`SYS_SPELL_END=29`）。
- dispatch 三臂。
- `sys_spell_timer`:`self_enemy_handle`（非敌 → 按无绑定，压 -1 不 Fault——读族误用降级
  同 SELF_HP 策略；实现：owner 非 ENEMY 直接 push -1）;敌 → `push(spell_frames_left_of(h))`。
- `sys_spell_begin`:逆序弹 7 参;`self_enemy_handle(task)?`（非敌 misuse → Fault）;
  `spell_begin_internal(...)` 返 true 且 `pattern >= 0` → spawn 模式任务
  （照 fire 样板:`sub_id`/`sub_meta`/`code_entry`/`tasks.spawn(owner=boss, parent=
  self_index+1)`;spawn 返回的 idx → `ctx.tasks.slots[idx].spell_bound = (slot+1) as u8`;
  pool full → pool_full[POOL_TASK] 计数）;pattern 越界号 → Fault（先校验，同 fire "坏号
  先查不建"）;begin 返 false → 不 spawn（P4-b 已在 internal 计数）。
- `sys_spell_end`:`self_enemy_handle`;`spell_end_by_owner(h)`。

**(c)** `vm.rs`:
- run_tasks owner 门禁（~437-450，敌/弹 owner 存活校验旁）加:
  `if t.spell_bound != 0 { let s = t.spell_bound as usize - 1; if s >= MAX_BOSSES ||
  body.spells[s].active == 0 { /* 同 owner 失效：静默杀，不发 fault */ kill; continue } }`
  （置于 owner 校验同款位置，池序确定；杀法照 owner 失效既有分支）。
- OP_SPAWN 继承（~345，spawn 落子后）:子任务 `spell_bound = task.spell_bound`（父继承）。
  **注意**:fire 的 task-spawn（syscall.rs）**不**继承——那里 spawn 的任务 spell_bound 保持
  0（弹任务不随卡死，spec §2.1）;spell_begin 的模式任务显式设 slot+1。三处 spawn 各自明确。

- [ ] **Step 4: 跑测确认绿 + 全绿 + Commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add -A ':!.superpowers'
git commit -m "feat(ecl): spell_begin/end/timer syscall + Task.spell_bound 随卡生死 + spawn 继承（刀 2/3）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

（刀 2 仍不改 rainbow;golden 只受 Task.spell_bound 字段平移，行数不变。）

---

### Task 3: 表层（builtins + wait_spell 糖 + rainbow 狗粮化 + 文档）

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（三内建 + 两测名单）
- Modify: `crates/stg-ecl-compiler/src/lang/`（wait_spell 语句糖——lex/parse/codegen 按既有
  `while` 语句糖机制加一条；具体模块以 lang 现有语句实现处为准）
- Modify: `crates/stg-harness/scenes/rainbow.ecl`（狗粮化）
- Modify: `docs/ecl-lang.md`（符卡节）、`docs/ecl-ops.md`（三号 + 糖注）
- Modify: `crates/stg-core/src/consts.rs` 无需（REQ 已在刀1）；
- 若 rainbow 用 `SPELL_WINDCHIME` const → 脚本内 `const` 声明（引擎不注册）

**Interfaces:**
- Consumes: Task 2 的 `syscall::{SYS_SPELL_BEGIN, SYS_SPELL_END, SYS_SPELL_TIMER}`;
  既有 `ParamKind::{Val, SubRef}`、`fire` 的 SubRef codegen（codegen.rs:575）、`while`/`wait`
  语句实现、rainbow.ecl 现结构（timer_ui/patrol/main）。
- Produces: 表层 `spell_begin(slot, id, pattern, time_limit, bonus0, flags, threshold)` /
  `spell_end()` / `spell_timer()->int` / `wait_spell()` 语句。

- [ ] **Step 1: 写失败测试**

- typeck/tests.rs:`spell_begin` 第三位 SubRef 接受 sub 名 / `none`;传表达式 → 报错（SubRef
  位不收求值表达式，同 fire task 参）;`spell_timer()` 用在需 int 处合法;`spell_end()` 只能
  做语句（ret None）。
- codegen e2e（本文件 run 助手）:内联脚本
  `sub p() { loop { wait(1); } } async sub boss() { spell_begin(0, 5, p, 60, 1000, 0, 0);
  wait_spell(); }` + 建 boss 敌 start_main → step → 断言：模式 p 起跑（弹/其效果可见）、
  到超时收卡（EVT/score）、`wait_spell` 糖展开（boss 任务在收卡后终止）。
- wait_spell 糖单测:编译 `wait_spell();` 与手写 `while spell_timer() >= 0 { wait(1); }`
  产出**同字节码**（糖=纯展开的判别）。

- [ ] **Step 2: 跑测确认红**

- [ ] **Step 3: 实现**

**(a)** builtins.rs 三条:
```rust
Builtin { name: "spell_begin", syscall: syscall::SYS_SPELL_BEGIN, is_op: false,
    params: &[Val(Int), Val(Int), Sub, Val(Int), Val(Int), Val(Int), Val(Int)], ret: None },
Builtin { name: "spell_end", syscall: syscall::SYS_SPELL_END, is_op: false,
    params: &[], ret: None },
Builtin { name: "spell_timer", syscall: syscall::SYS_SPELL_TIMER, is_op: false,
    params: &[], ret: Some(Int) },
```
两测名单（lookup_finds_every / void_builtins_have_none）加 `spell_begin`/`spell_end`
（void）+ `spell_timer`（有返回，只加 lookup 名单）。

**(b)** wait_spell 语句糖:照 lang 既有 `wait(n)`/`while` 的实现路径，加一条内建语句
`wait_spell()` → parser 识别 → codegen 展开为 `while spell_timer() >= 0 { wait(1); }`
的等价字节码（复用 while/比较/spell_timer 调用的既有发射）。**实现处以 lang 现有语句糖
锚点为准**（parser 的语句分派 + codegen 的语句 emit）；若结构上更适合做成"parser 直接
展开成 while AST 节点"，取更小改动的一种，报告说明。

**(c)** rainbow.ecl 狗粮化:删 `timer_ui` sub + main 里对它的 spawn/等待循环;boss 主控
（现结构以文件为准）改为 `spell_begin(0, SPELL_WINDCHIME, <原弹幕sub>, <时限>, <bonus>,
0, 0); wait_spell();`;`const SPELL_WINDCHIME: int = <值>;` 脚本内声明。保持单卡、行为等价
到"能收卡"即可（金向量本就要变，不追旧流）。

**(d)** 文档:ecl-lang.md 新「符卡」节（两行范式 + 参数表 + 模式随卡生死说明 + wait_spell
糖 + 卡 id const 约定）;ecl-ops.md 号表三行 + wait_spell 糖注。

- [ ] **Step 4: 全绿 + 金向量（行为变，记录新流行数/退出码）+ Commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t3.txt
wc -l .superpowers/golden-post-t3.txt   # 行数应仍 = 段数×帧数（脚本变但帧数不变）；退出 0
cargo run --release -q -p stg-harness -- storm && echo STORM-OK   # spells 字段随快照往返
git add -A ':!.superpowers'
git commit -m "feat(ecl-lang): spell_begin/end/timer 内建 + wait_spell 糖 + rainbow 符卡狗粮化（刀 3/3）

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 合入前收尾（控制器步骤）

1. `stg-world-design.md` A2 段加范围修订注记一行（记账归引擎/控制归脚本，spec 2026-07-24）;
   D12 表加三 syscall 行。
2. `CLAUDE.md` 仓库结构图加 `spell.rs`;`docs/follow-ups.md` 记 G1（读资源 syscall 降级）+
   ZUN 分段衰减候选。
3. `docs/architecture.md` 若有 boss/符卡相关行则更新。
4. `PROGRESS.md` 史加一行 + 「现在」段（符卡机构落地，两线内容层前置就绪）。
