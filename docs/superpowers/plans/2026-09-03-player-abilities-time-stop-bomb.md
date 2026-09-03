# 自机能力刀（时间停止 + bomb）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给引擎加两件自机能力——时间停止（双向：玩家停敌 / ECL 停玩家）与 bomb（无敌 + 全屏消弹 + 范围伤害 + 全屏吸道具 + deathbomb）。

**Architecture:** 时停 = `WorldBody.freeze_left: [u16; 2]` 两个倒计时，冻结掩码由它**推导**（不存）；三组分区 A（自机主动行为）/ B（自机弹飞行）/ C（世界演化与裁决）。门禁写在**各相位函数体内、`phase_enter` 之后**——不在 `step.rs` 拦，因为跳过相位函数会让 debug 的 PhaseGuard 断言当场失败，而且 §3.5 宪法顺序归组装层（P2）、本刀不该动它。bomb 复用既有机制（`invuln` / `attract_all_items` / `FieldPool`），效果描述走表驱动 `CharacterCfg.bomb: BombCfg`。

**Tech Stack:** Rust 1.94.0（edition 2024）· `stg-core`（断层线以下，无浮点/时钟/宿主 RNG）· `stg-ecl-compiler` · `stg-harness`。

**Spec:** [`docs/superpowers/specs/2026-09-03-time-stop-design.md`](../specs/2026-09-03-time-stop-design.md)

## Global Constraints

- **I1 数值**：断层线以下唯一标量是 `Fx`（Q16.16 i32）；不得出现 `f32`/`f64`。
- **I4 顺序**：一切遍历按池索引升序；不得引入无序容器。
- **I6 时间**：一切计时用整数帧。
- **I7 布局**：`World` 内无指针/引用/堆容器；新字段必须是 POD 且全零合法。
- **P4 错误三铁律**：资源耗尽 → 确定性降级不 panic；调用方违约 → 确定性安全结果 + 计数；引擎自身 bug → debug 帧内断言。
- **P5 无回调**：world 不持有回调/函数指针/注册表；定制行为只能是**数据**。
- **P6 全量校验**：住进 `World` 的字段一律入校验和，无例外。
- **新增 `World` 字段必做四件**：① `copy_into` 清单（`define_pool!` 生成的池无需手动同步，手写 `impl` 必须两侧同步）② checksum（derive 默认全量入，skip 须给理由）③ D10 容量预算 ④ SaveBytes。`world_size_sentinel_guards_copy_into_field_list` 会红，**先按它自己的注释清单逐条判定，再更新数字**。
- **哨兵数字一律填实测输出，不许手算。**
- **每个 Task 结束前跑**：`cargo fmt --all` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo clippy --release --workspace --all-targets -- -D warnings` · `cargo test --workspace`。
- **金向量本刀预期改变**（`World` 变宽 ⇒ 空字段从帧 0 进哈希）。不是回归，不要试图"修回去"。最终 md5 在 Task 9 记录。

---

### Task 1: `freeze_left` 状态、掩码读口与倒计时

**Files:**
- Modify: `crates/stg-core/src/world.rs`（`WorldBody` 字段区，`bg_phase_frame` 之后 / `reqs` 之前；`begin()`；新增三个掩码读口）
- Modify: `crates/stg-core/src/world/view.rs`（`WorldView` 加只读口）
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER` 14 → 15 + 文档）
- Modify: `crates/stg-core/src/step.rs`（两条哨兵测试 + `engine_ver_anchored`）
- Test: `crates/stg-core/src/world.rs` 的 `mod tests`

**Interfaces:**
- Produces: `WorldBody::actor_frozen(&self) -> bool` · `WorldBody::scene_frozen(&self) -> bool` · `WorldBody::shots_frozen(&self) -> bool`（均 `pub(crate)`）；`WorldBody.freeze_left: [u16; 2]`（`pub(crate)`）；`WorldView::freeze_left(self) -> [u16; 2]`（`pub`）。
- Consumes: 无。

- [ ] **Step 1: 写失败测试**

加到 `crates/stg-core/src/world.rs` 的 `mod tests`：

```rust
/// 倒计时挂**真实帧**、不属于任何冻结组——两边同时开时若各自跟组走会互相冻死
/// （spec §5「死锁与它的解」）。本条守的就是"它在相位 0 无条件递减"。
#[test]
fn freeze_countdowns_tick_in_begin_unconditionally() {
    let mut w = crate::step::World::new(1);
    w.body.freeze_left = [2, 3];
    // ⚠️ `begin()` 头一句是 `phase_enter(PH_BEGIN)`，它在 debug 下断言
    // `phase_guard == 0` 并推进。连调多次必须每次把护栏拨回相位 0——本模块其余
    // 直调相位函数的测试是同款写法。
    let mut tick = |w: &mut crate::step::World| {
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_BEGIN;
        }
        w.body.begin();
    };
    tick(&mut w);
    assert_eq!(w.body.freeze_left, [1, 2], "两个倒计时都必须在相位 0 递减");
    tick(&mut w);
    assert_eq!(w.body.freeze_left, [0, 1]);
    tick(&mut w);
    assert_eq!(w.body.freeze_left, [0, 0], "到 0 后饱和，不回绕");
}

/// 掩码是**推导**的：A 冻 ⇔ left[1]>0、C 冻 ⇔ left[0]>0、B 冻 ⇔ 任一 >0。
/// 判别力：四种组合逐个断言——只测"全零"与"全非零"的话，把 A/C 写反照样绿。
#[test]
fn freeze_mask_is_derived_from_the_two_countdowns() {
    let mut w = crate::step::World::new(1);
    let cases = [
        ([0u16, 0u16], (false, false, false)),
        ([5, 0], (false, true, true)),   // 玩家技能：冻 B+C，A 跑
        ([0, 5], (true, false, true)),   // ECL 演出：冻 A+B，C 跑
        ([5, 5], (true, true, true)),    // 全场静止
    ];
    for (left, (a, c, b)) in cases {
        w.body.freeze_left = left;
        assert_eq!(w.body.actor_frozen(), a, "actor @ {left:?}");
        assert_eq!(w.body.scene_frozen(), c, "scene @ {left:?}");
        assert_eq!(w.body.shots_frozen(), b, "shots @ {left:?}");
    }
}

/// P6：新字段必须进校验和（derive 默认全量入，本条是它的可观测面）。
#[test]
fn freeze_left_enters_the_checksum() {
    use crate::checksum::Checksum;
    let mut w = crate::step::World::new(1);
    let base = w.checksum();
    w.body.freeze_left[0] = 1;
    assert_ne!(w.checksum(), base, "freeze_left 必须入校验和");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib freeze_ 2>&1 | tail -20`
Expected: 编译失败——`no field 'freeze_left' on type 'WorldBody'`。

- [ ] **Step 3: 加字段与读口**

`crates/stg-core/src/world.rs`，把字段插在 `pub(crate) bg_phase_frame: u32,` 之后、`#[checksum(skip = ...)] pub(crate) reqs: ...` 之前：

```rust
    /// 时停剩余帧（自机能力刀，2026-09-03）：`[0]` = 玩家技能（冻 B+C）、
    /// `[1]` = ECL 演出（冻 A+B）。**冻结掩码是推导的、不存**——见
    /// [`Self::actor_frozen`]/[`Self::scene_frozen`]/[`Self::shots_frozen`]；
    /// 存一份掩码只会多一个与倒计时不同步的机会。
    ///
    /// **两个都在相位 0 `begin` 无条件递减**，不属于任何冻结组：若各自跟组走，
    /// 两边同时开启时 A 被演出冻住 ⇒ 技能倒计时不走、C 被技能冻住 ⇒ 演出倒计时
    /// 不走，**世界永远解不开**（spec §5）。
    pub(crate) freeze_left: [u16; 2],
```

在 `impl WorldBody`（`begin` 附近）加三个读口：

```rust
    /// A 组（自机主动行为：移动/发弹/用能力）是否冻结 —— ECL 演出（`freeze_left[1]`）。
    pub(crate) fn actor_frozen(&self) -> bool {
        self.freeze_left[1] > 0
    }
    /// C 组（世界演化与裁决：敌/弹/ECL/道具/作用区/背景/自机被动计时/相位 6·7）
    /// 是否冻结 —— 玩家技能（`freeze_left[0]`）。
    pub(crate) fn scene_frozen(&self) -> bool {
        self.freeze_left[0] > 0
    }
    /// B 组（自机弹的飞行）是否冻结 —— **任一方向的时停都冻它**。这不是巧合：
    /// 弹一旦离开枪口就不再属于自机（spec §3）。
    pub(crate) fn shots_frozen(&self) -> bool {
        self.freeze_left[0] > 0 || self.freeze_left[1] > 0
    }
```

改 `begin()`：

```rust
    pub(crate) fn begin(&mut self) {
        self.phase_enter(PH_BEGIN);
        self.hits_len = 0;
        self.frame_events_len = 0;
        self.reqs_len = 0;
        // 时停倒计时挂**真实帧**、不属于任何冻结组（spec §5 的死锁解）。
        self.freeze_left[0] = self.freeze_left[0].saturating_sub(1);
        self.freeze_left[1] = self.freeze_left[1].saturating_sub(1);
    }
```

- [ ] **Step 4: 加 `WorldView` 只读口**

`crates/stg-core/src/world/view.rs`，紧跟 `bg_phase_frame()` 之后：

```rust
    /// 时停剩余帧只读口（自机能力刀）：`[0]` = 玩家技能、`[1]` = ECL 演出。
    /// 表现层据此画停时画面效果。
    pub fn freeze_left(self) -> [u16; 2] {
        self.body.freeze_left
    }
```

- [ ] **Step 5: 跑测试确认通过（哨兵会红，下一步处理）**

Run: `cargo test -p stg-core --lib freeze_ 2>&1 | tail -10`
Expected: 三条 PASS。

Run: `cargo test -p stg-core --lib sentinel 2>&1 | grep -E "left:|right:"`
Expected: `world_size_sentinel_guards_copy_into_field_list` 红，打出实测的新 `(WorldBody, World)` 二元组。

- [ ] **Step 6: 按哨兵自己的清单更新它**

在 `crates/stg-core/src/step.rs` 的 `world_size_sentinel_guards_copy_into_field_list` 里，`const EXPECTED` 之前追加一段说明（**四项逐条判定**），然后把两个 `EXPECTED` 改成**上一步打出来的实测值**：

```rust
        // 2026-09-03（自机能力刀 Task 1）：`WorldBody` 新增 `freeze_left: [u16; 2]`
        // （4 B，插在 `bg_phase_frame` 与 `reqs` 之间，两侧均 4 对齐 ⇒ 无对齐吸收）。
        // ① `copy_into` 手写清单**必须同步**加 `dst.freeze_left = self.freeze_left;`
        //    ——它不是池、不走 `define_pool!` 生成；② checksum 走 derive 默认全量入
        //    （未加 skip，判别面 = `freeze_left_enters_the_checksum`）；③ D10 容量预算：
        //    非池 cap 变更，标量 4 B，`stg-world-design.md` D10 表已加行；
        //    ④ SaveBytes 走 `WorldBody` 的 derive 自动入 ⇒ 存档 wire format 变化，
        //    故 `ENGINE_VER` 14→15（见 lib.rs）。
```

⚠️ **同时改 `copy_into`（必做，位置已核实）**：手写逐字段清单在
`crates/stg-core/src/step.rs:153` 的 `World::copy_into`（**不在 `world.rs`**）。在
`d.bg_phase_frame = s.bg_phase_frame;` 之后加一行：

```rust
        d.freeze_left = s.freeze_left;
```

**漏拷编译不报错**——这正是那条尺寸哨兵存在的全部理由。

- [ ] **Step 7: 更新池账目文档与 D10 表**

`stg-world-design.md` 的 D10 容量表，在「自机×2 / boss_ui×2 …」那一行的枚举里追加 `/ freeze_left（时停倒计时 [u16;2]，4 B，2026-09-03 自机能力刀）`。

- [ ] **Step 8: bump `ENGINE_VER` 并更新锚点测试**

`crates/stg-core/src/lib.rs`，在 `13 → 14` 那段之后、`pub const ENGINE_VER` 之前追加：

```rust
/// **14 → 15**（自机能力刀：时间停止 + bomb，2026-09-03）：**布局 + 号表 + 输入词表
/// 三重变更**，任一即足以 bump。① `World` 变宽（`WorldBody.freeze_left` 4 B +
/// `PlayerState.time_stops` 1 B×2）⇒ 快照与存档 wire format 变，旧存档尺寸对不上，
/// 是响亮失败；② syscall 号表新增 `513 add_time_stops` / `560 time_stop_player`；
/// ③ 输入动作词表新增 `BTN_TIMESTOP = 7`（位=0 等价旧行为，故非回放破坏性变更，
/// 但 `actions_vocab_hash` 变）；④ `WorldTables` 新增 `CharacterCfg.bomb`
/// ⇒ 表 `content_hash` 变 ⇒ 身份三元组变。
///
/// **金向量预期改变**（新字段进哈希，同道具池刀的道理）：形态可解释，不是行为回归。
pub const ENGINE_VER: u32 = 15;
```

改 `crates/stg-core/src/step.rs` 的 `engine_ver_anchored`：把 `13` 改成 `15`，并把新的理由**前置**到消息串开头（保留旧文作为"前一次"）：

```rust
            crate::ENGINE_VER,
            15,
            "bump 必须是有意识决定(评审 + 改本测试)——14→15：自机能力刀(时间停止 + bomb)。\
             **布局 + 号表 + 输入词表三重变更**:World 变宽(freeze_left 4B + time_stops 1B×2)\
             ⇒ 旧存档尺寸对不上、响亮失败;号表新增 513 add_time_stops / 560 \
             time_stop_player;输入词表新增 BTN_TIMESTOP=7(位=0 等价旧行为);WorldTables \
             新增 CharacterCfg.bomb ⇒ 表 content_hash 变。金向量预期改变(新字段进哈希)。\
             前一次 13→14：道具池 cap 512→1024\
```

（保留原串剩余部分不动。）

- [ ] **Step 9: 跑全套并提交**

```bash
cargo fmt --all
cargo test --workspace 2>&1 | grep -E "^test result"
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --release --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(core): 时停世界侧状态 freeze_left + 推导掩码 + ENGINE_VER 14→15

三组分区（A 自机主动行为 / B 自机弹飞行 / C 世界演化与裁决）的世界侧载体。
掩码推导不存——存一份只会多一个与倒计时不同步的机会。两个倒计时在相位 0
无条件递减：若各自跟组走，两边同时开时会互相冻死、世界永远解不开。

本 Task 只加状态与读口，相位门禁在 Task 3。"
```

---

### Task 2: `integrate` 的零行为变更提取

**Files:**
- Modify: `crates/stg-core/src/world/integrate.rs`

**Interfaces:**
- Produces: `WorldBody::integrate_bullets(&mut self)` · `integrate_shots(&mut self)` · `integrate_enemies(&mut self, tables: &WorldTables)` · `integrate_items(&mut self, tables: &WorldTables)` · `integrate_fields(&mut self)`（均 `fn`，私有）。
- Consumes: 无。

**为什么单开一个 Task**：相位 5 是唯一"一半 B 一半 C"的相位，直接在原地插 `if` 会把三百行缩进重排、review 里看不出真正的改动。先做**零行为变更**的提取并用金向量证明它零变更，门禁才好加。

- [ ] **Step 1: 记录改动前的金向量指纹**

```bash
cargo run -q -p stg-harness -- golden --out /tmp/g-before.txt && md5sum /tmp/g-before.txt
```
记下这个 md5（它已经含 Task 1 的字段，与 main 不同，这是预期的）。

- [ ] **Step 2: 提取五个私有函数**

把 `integrate()` 的函数体按既有注释边界切成五段，**内容一字不改、顺序一字不改**（模块
文档写死了冻结趟序：弹 → 自机弹 → 敌人 → 道具 → 作用区）。

**切点就是那五条既有注释**（写本计划时的行号，编辑后会漂，以注释文本为准）：

| 段 | 起点（注释原文） | 当前行 |
|---|---|---|
| ① 弹 | `phase_enter(super::PH_INTEGRATE);` 的下一行起 | 20 |
| ② 自机弹 | `// 自机弹：pos += vel（无 delay/life）` | 58 |
| ③ 敌人 | `// 敌人：分层 —— ① 速度插值恒跑，…` | 69 |
| ④ 道具 | `// 道具（D7）：触发判定先于移动；…` | 133 |
| ⑤ 作用区 | `// 作用区：寿命倒数（照抄弹的模式；…` | 147 |

每段连同它头上的注释一起搬进对应的私有函数；`integrate()` 只剩五句调用：

```rust
    pub(crate) fn integrate(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_INTEGRATE);
        // 冻结趟序（stg-world-design.md:168）：弹 → 自机弹 → 敌人 → 道具 → 作用区。
        // **顺序是宪法，别重排**——五个私有函数只是把原函数体切开，零行为变更
        // （自机能力刀 Task 2；门禁在 Task 3 加）。
        self.integrate_bullets();
        self.integrate_shots();
        self.integrate_enemies(tables);
        self.integrate_items(tables);
        self.integrate_fields();
    }

    /// 相位 5 趟一：弹（delay 门 → 模式效果 → pos+=vel → 反弹 → life）。
    fn integrate_bullets(&mut self) { /* ① 原第 20-57 行，原样搬入 */ }

    /// 相位 5 趟二：自机弹（pos += vel；无 delay/life）。
    fn integrate_shots(&mut self) { /* ② 原第 58-68 行 */ }

    /// 相位 5 趟三：敌人（速度插值 / 位置插值 / pos+=vel / invuln / hit_flash）。
    fn integrate_enemies(&mut self, tables: &WorldTables) { /* ③ 原第 69-132 行 */ }

    /// 相位 5 趟四：道具（触发判定先于移动）。
    fn integrate_items(&mut self, tables: &WorldTables) { /* ④ 原第 133-146 行 */ }

    /// 相位 5 趟五：作用区 life 倒数。
    fn integrate_fields(&mut self) { /* ⑤ 原第 147 行至函数末 */ }
```

若某段实际不用 `tables`，就不要给它加参数（clippy 的 `-D warnings` 会抓未用参数）。

- [ ] **Step 3: 证明零行为变更**

```bash
cargo test --workspace 2>&1 | grep -E "^test result"
cargo run -q -p stg-harness -- golden --out /tmp/g-after.txt && md5sum /tmp/g-after.txt
diff /tmp/g-before.txt /tmp/g-after.txt && echo "金向量逐字节相同 ✔"
```
Expected: 全绿 + **两个 md5 相同**。不相同就是提取时改了顺序或漏了一段，回去查，**不要往下走**。

- [ ] **Step 4: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "refactor(core): 相位 5 切成五个私有趟函数（零行为变更）

为 Task 3 的分组门禁做准备——相位 5 是唯一一半 B 一半 C 的相位，原地插
if 会把三百行缩进重排、review 里看不出真改动。顺序一字未动（冻结趟序是
宪法）；金向量逐字节相同为证。"
```

---

### Task 3: 相位门禁接线

**Files:**
- Modify: `crates/stg-core/src/step.rs`（**仅**相位 2 的 `run_tasks` 包一层）
- Modify: `crates/stg-core/src/world/player.rs`（`update_players` 按 A/C 拆）
- Modify: `crates/stg-core/src/world/integrate.rs`（按 B/C 门禁）
- Modify: `crates/stg-core/src/world/transform.rs`、`collide.rs`、`settle.rs`、`cleanup.rs`（各加一句早退）
- Modify: `crates/stg-core/src/world.rs`（`advance()` 的 `bg_phase_frame`）
- Test: `crates/stg-core/src/step.rs` 的 `mod tests`

**Interfaces:**
- Consumes: `WorldBody::actor_frozen/scene_frozen/shots_frozen`（Task 1）；`integrate_*` 五函数（Task 2）。
- Produces: 无新公开面。

**关键约束（先读再动手）**：`phase_enter(p)` 在 debug 下断言 `phase_guard == p` 并推进。**跳过相位函数会让下一相的断言当场失败**。所以门禁一律写在**相位函数体内、`phase_enter` 之后**，绝不在 `step.rs` 里跳过调用。唯一的例外是相位 2 的 `run_tasks`——它不是 `WorldBody` 方法，`phase_enter(PH_DIRECTOR)` 由 `step.rs` 自己调，包一层不影响护栏。

- [ ] **Step 1: 写失败测试（主测 + 六条腿）**

加到 `crates/stg-core/src/step.rs` 的 `mod tests`：

```rust
/// 造一个"什么都在动"的世界：弹在飞、敌在动、道具在落、作用区在倒数、自机在移动。
/// 五类都要有，否则主测的观测面是空的、押不住任何东西。
fn busy_world() -> Box<World> {
    use crate::field::FIELD_CLEAR_BULLETS;
    let mut w = World::new(1);
    // 弹（斜飞，两轴都动）
    for k in 0..3 {
        let h = crate::world::test_support::bullet_at(&mut w, 10 * k, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.vx[i] = Fx::from_int(1);
        w.body.bullets.vy[i] = Fx::from_int(2);
    }
    // 自机弹
    w.body.create_player_shot(crate::shots::ShotInit {
        x: Fx::ZERO,
        y: Fx::from_int(300),
        vx: Fx::ZERO,
        vy: Fx::from_int(-8),
        damage: 1,
        radius: Fx::from_int(4),
        sprite: 0,
        owner: 0,
    });
    // 敌（有速度）
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 50, 100);
    let ei = w.body.enemies.get(eh).unwrap();
    w.body.enemies.vy[ei] = Fx::from_int(1);
    // 道具（会下落）
    w.body.spawn_drop(Fx::ZERO, Fx::from_int(60), crate::items::ITEM_POINT, &TABLES_V0);
    // 作用区（life 会倒数）
    w.body.create_field(crate::field::FieldInit {
        x: Fx::ZERO,
        y: Fx::from_int(224),
        radius: Fx::from_int(10),
        dmg_per_frame: 0,
        life: 600,
        owner: 0,
        flags: FIELD_CLEAR_BULLETS,
    });
    w
}

fn step_empty(w: &mut World) {
    crate::step::step(
        w,
        &TABLES_V0,
        &crate::ecl::image::EclImage::empty(),
        &InputFrame::empty(w.frame()),
    );
}

/// **主测**：全场静止（两位都开）跑一帧，除 `frame` 与 `freeze_left` 外**整块 World
/// 逐位不变**。抹掉那两个"恒跑"字段后比整块校验和。
///
/// 杀手级性质：**将来谁往 World 加新计时器而忘了裁定，本条自动照出来**——新字段由
/// `#[derive(Checksum)]` 自动进哈希（P6），不需要有人回来补断言。
#[test]
fn full_freeze_changes_nothing_but_the_always_running_fields() {
    use crate::checksum::Checksum;
    let mut w = busy_world();
    let mut before = World::new(0);
    w.copy_into(&mut before);

    w.body.freeze_left = [10, 10];
    step_empty(&mut w);

    w.body.frame = before.body.frame;
    w.body.freeze_left = before.body.freeze_left;
    assert_eq!(
        w.checksum(),
        before.checksum(),
        "全场静止下除恒跑字段外不得有任何变化（有东西变了 = 某个计时器漏了裁定）"
    );
}

/// 玩家技能（冻 B+C）：C 组停、A 组跑。判别力=两侧都断言，只断一侧的话
/// "门禁写成恒冻一切"或"恒不冻"各能骗过其中一条。
#[test]
fn player_skill_freezes_scene_but_not_the_actor() {
    let mut w = busy_world();
    let bullet_y = w.body.bullets.y[0];
    let enemy_i = w.body.enemies.iter_alive().next().unwrap();
    let enemy_y = w.body.enemies.y[enemy_i];
    let shot_y = w.body.shots.y[0];
    w.body.players[0].x = Fx::ZERO;

    w.body.freeze_left = [10, 0];
    let mut input = InputFrame::empty(w.frame());
    input.actions[0].buttons = crate::input::BTN_RIGHT;
    crate::step::step(&mut w, &TABLES_V0, &crate::ecl::image::EclImage::empty(), &input);

    assert_eq!(w.body.bullets.y[0], bullet_y, "C 组：敌弹必须冻住");
    assert_eq!(w.body.enemies.y[enemy_i], enemy_y, "C 组：敌人必须冻住");
    assert_eq!(w.body.shots.y[0], shot_y, "B 组：自机弹必须冻住");
    assert_ne!(w.body.players[0].x, Fx::ZERO, "A 组：自机必须还能移动");
}

/// ECL 演出（冻 A+B）：A 组停、C 组跑。与上一条互为镜像。
#[test]
fn ecl_cutscene_freezes_the_actor_but_not_the_scene() {
    let mut w = busy_world();
    let bullet_y = w.body.bullets.y[0];
    let shot_y = w.body.shots.y[0];
    w.body.players[0].x = Fx::ZERO;

    w.body.freeze_left = [0, 10];
    let mut input = InputFrame::empty(w.frame());
    input.actions[0].buttons = crate::input::BTN_RIGHT;
    crate::step::step(&mut w, &TABLES_V0, &crate::ecl::image::EclImage::empty(), &input);

    assert_ne!(w.body.bullets.y[0], bullet_y, "C 组：敌弹必须照飞");
    assert_eq!(w.body.shots.y[0], shot_y, "B 组：自机弹仍要冻住（两个方向都冻 B）");
    assert_eq!(w.body.players[0].x, Fx::ZERO, "A 组：自机必须被定住");
}

/// **规则不能照搬**（spec §4）：门禁挂在 C 组、不是"是否冻结"。ECL 演出把自机定住时
/// 相位 6/7 照跑 ⇒ 自机**照样会被打死**。写成"冻结即免伤"这条当场红——它守的正是
/// 那条推导出来的门禁规则，没有它整个演出会变得毫无威胁。
#[test]
fn cutscene_freeze_still_lets_the_player_be_hit() {
    let mut w = World::new(1);
    w.body.players[0].x = Fx::ZERO;
    w.body.players[0].y = Fx::from_int(384);
    crate::world::test_support::bullet_at(&mut w, 0, 384);
    w.body.freeze_left = [0, 10]; // 冻 A+B，C 跑
    step_empty(&mut w);
    assert_eq!(
        w.body.players[0].life_state,
        crate::player::LIFE_DEATHWINDOW,
        "定住玩家的演出期间碰撞照跑，自机该被打进决死窗口"
    );
}

/// 玩家技能期间相位 6/7 不跑 ⇒ 撞进冻结的弹里也不死（绝对安全窗，裁定 #3）。
#[test]
fn player_skill_freeze_makes_the_player_untouchable() {
    let mut w = World::new(1);
    w.body.players[0].x = Fx::ZERO;
    w.body.players[0].y = Fx::from_int(384);
    crate::world::test_support::bullet_at(&mut w, 0, 384);
    w.body.freeze_left = [10, 0]; // 冻 B+C
    step_empty(&mut w);
    assert_eq!(
        w.body.players[0].life_state,
        crate::player::LIFE_ALIVE,
        "冻 C 时相位 6/7 不跑，重合也不该判中弹"
    );
}

/// 背景停滞不是"什么都不做"就有的：背景由 `frame − bg_phase_frame` 驱动而 frame 恒增，
/// 只冻别的会让背景照走。判别力=断言那个**差值**不变，而不是断言某个字段不变。
#[test]
fn scene_freeze_keeps_the_background_elapsed_time_still() {
    let mut w = World::new(1);
    step_empty(&mut w);
    let elapsed = |w: &World| w.body.frame - w.body.bg_phase_frame;
    let e0 = elapsed(&w);
    w.body.freeze_left = [10, 0];
    step_empty(&mut w);
    assert_eq!(elapsed(&w), e0, "冻 C 时背景经过的时间不得增长");
    w.body.freeze_left = [0, 0];
    step_empty(&mut w);
    assert_eq!(elapsed(&w), e0 + 1, "解除后背景恢复走时");
}

/// **死锁腿**（spec §5）：两边同时开必须都能解除。把任一倒计时挪到组门禁之后 ⇒ 这条
/// 变成死循环/超时。取 N=1 与 N=2 两点：只测 N=1 的话"恒冻一帧"的错实现照样绿
/// （ENGINE_VER 11→12 的 `wait` 差一帧就是这么被咬的）。
#[test]
fn both_freezes_always_expire_and_last_exactly_n_frames() {
    for n in [1u16, 2] {
        let mut w = busy_world();
        let y0 = w.body.bullets.y[0];
        w.body.freeze_left = [n, n];
        for _ in 0..n {
            step_empty(&mut w);
            assert_eq!(w.body.bullets.y[0], y0, "n={n}：冻结期间不得移动");
        }
        step_empty(&mut w);
        assert_ne!(w.body.bullets.y[0], y0, "n={n}：第 n+1 帧必须已解除");
        assert_eq!(w.body.freeze_left, [0, 0], "n={n}：两个倒计时都必须归零");
    }
}
```

/// **符卡不白嫖**（spec §4 的白送后果）：符卡计时住相位 7 尾，冻 C ⇒ 相位 7 不跑 ⇒
/// 时停期间符卡**不倒计时**，没法用时停拖过 survival 卡。
/// 判别力=断言"恰好少走 N"而不是"变小了"：门禁若漏了相位 7，frames_left 会照常走。
#[test]
fn player_skill_freeze_does_not_burn_spell_time() {
    let mut w = World::new(1);
    let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
    // 参数序同 spell.rs 测试的既有用法：(slot, boss, spell_id, hp_threshold,
    // hp_start, flags, frames)。**先读 spell.rs 确认签名没变。**
    assert!(w.body.spell_begin_internal(0, boss, 1, 100, 1000, 0, 300));
    step_empty(&mut w);
    let left0 = w.body.spells[0].frames_left;

    const N: u16 = 5;
    w.body.freeze_left = [N, 0];
    for _ in 0..N {
        step_empty(&mut w);
    }
    assert_eq!(w.body.spells[0].frames_left, left0, "冻 C 期间符卡不得倒计时");
    step_empty(&mut w);
    assert_eq!(w.body.spells[0].frames_left, left0 - 1, "解除后恢复走时");
}

/// **蓄水池**（本机制的核心效果）：时停中按住射击 ⇒ 自机弹**数量增长**（发弹在 A 组、
/// 照跑）而**每颗坐标逐位不变**（飞行在 B 组、冻住）。
/// 判别力=两件都断：只断数量的话"弹照飞"也绿；只断坐标的话"根本没发出来"也绿。
#[test]
fn time_stop_stockpiles_frozen_player_shots() {
    let mut w = World::new(1);
    w.body.freeze_left = [60, 0];
    let mut fire = |w: &mut World| {
        let mut input = InputFrame::empty(w.frame());
        input.actions[0].buttons = crate::input::BTN_SHOT;
        crate::step::step(w, &TABLES_V0, &crate::ecl::image::EclImage::empty(), &input);
    };
    fire(&mut w);
    let n1 = w.body.shots.iter_alive().count();
    assert!(n1 > 0, "发弹在 A 组，时停期间照常产出");
    let snapshot: Vec<(Fx, Fx)> = w
        .body
        .shots
        .iter_alive()
        .map(|i| (w.body.shots.x[i], w.body.shots.y[i]))
        .collect();

    for _ in 0..20 {
        fire(&mut w);
    }
    assert!(w.body.shots.iter_alive().count() > n1, "弹应持续堆积");
    // 头 n1 颗（低索引，I4 分配序）必须一动没动
    for (k, &(x, y)) in snapshot.iter().enumerate() {
        let i = w.body.shots.iter_alive().nth(k).unwrap();
        assert_eq!((w.body.shots.x[i], w.body.shots.y[i]), (x, y), "第 {k} 颗必须冻在出发点");
    }
}

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib freeze 2>&1 | tail -20`
Expected: 主测与 `player_skill_*` / `cutscene_*` / `scene_freeze_*` / `both_freezes_*` 全部 FAIL（门禁还没写，什么都没冻）。
`cutscene_freeze_still_lets_the_player_be_hit` 应当**已经 PASS**（现在什么都不冻，碰撞本来就跑）——这条是防回归的。

- [ ] **Step 3: 相位 2 —— `step.rs` 唯一的改动**

`crates/stg-core/src/step.rs` 的 `step_with_director`：

```rust
    b.phase_enter(PH_DIRECTOR); // 2：导演槽（护栏在组装层押）
    // C 组门禁：全部 ECL 任务的 owner 只有 STAGE/ENEMY/BULLET（无 PLAYER）⇒ 冻 C 就是
    // 整条跳过，不必逐任务筛 owner。导演闭包**照跑**——它是宿主的槽、不是世界的一部分。
    if !b.scene_frozen() {
        crate::ecl::vm::run_tasks(&mut world.tasks, b, ecl, tables);
    }
    director(b);
```

**其余相位一行不改**——§3.5 宪法顺序归组装层（P2），门禁写在相位函数体内。

- [ ] **Step 4: 相位 3 —— `update_players` 按 A/C 拆**

`crates/stg-core/src/world/player.rs`，把 `update_players` 改成：

```rust
    pub(crate) fn update_players(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_PLAYERS);
        let scene = self.scene_frozen();
        let actor = self.actor_frozen();
        for i in 0..crate::MAX_PLAYERS {
            if matches!(self.players[i].life_state, LIFE_ABSENT | LIFE_GAMEOVER) {
                continue;
            }
            // ── C 组：世界对自机的**裁决**（生死状态机计时）。归 C 而非 A 是有理由的
            //    ——它不是自机的行动。放 A 会造出"演出定住你、你中弹进决死窗口而窗口
            //    计时被冻、又不能 bomb ⇒ 永远挂在决死窗里"的怪状态（spec §3）。
            if !scene {
                match self.players[i].life_state {
                    LIFE_DEATHWINDOW => {
                        if self.players[i].state_timer > 0 {
                            self.players[i].state_timer -= 1;
                        }
                        if self.players[i].state_timer == 0 {
                            self.commit_death(i);
                        }
                    }
                    LIFE_RESPAWNING => {
                        if self.players[i].invuln > 0 {
                            self.players[i].invuln -= 1;
                        }
                        if self.players[i].invuln == 0 {
                            self.players[i].life_state = LIFE_ALIVE;
                        }
                    }
                    LIFE_ALIVE => {
                        if self.players[i].invuln > 0 {
                            self.players[i].invuln -= 1;
                        }
                    }
                    _ => {}
                }
                // commit_death 可能刚把 lives 耗尽置 GAMEOVER → 再判一次跳过移动/发弹
                if self.players[i].life_state == LIFE_GAMEOVER {
                    continue;
                }
            }
            // ── A 组：自机的**主动行为**。
            if actor {
                continue;
            }
            self.move_player(i, tables);
            #[allow(clippy::single_match)]
            match self.players[i].character_id {
                0 => self.char0_update_shot(i, tables),
                _ => {}
            }
        }
    }
```

⚠️ **注意行为等价**：原实现的 `LIFE_ABSENT | LIFE_GAMEOVER => continue` 是 `match` 的一个臂；提到循环开头是等价的（那两态原本就直接 `continue`），但**必须保留 `commit_death` 之后的 GAMEOVER 复查**，否则刚耗尽的自机会在同一帧继续移动发弹。

- [ ] **Step 5: 相位 5 —— `integrate` 按 B/C 门禁**

`crates/stg-core/src/world/integrate.rs`：

```rust
    pub(crate) fn integrate(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_INTEGRATE);
        // 冻结趟序（stg-world-design.md:168）：弹 → 自机弹 → 敌人 → 道具 → 作用区。
        // **顺序是宪法，门禁不得重排它**——只在原位加条件。
        let scene = self.scene_frozen();
        if !scene {
            self.integrate_bullets();
        }
        if !self.shots_frozen() {
            self.integrate_shots();
        }
        if !scene {
            self.integrate_enemies(tables);
            self.integrate_items(tables);
            self.integrate_fields();
        }
    }
```

- [ ] **Step 6: 相位 4 / 6 / 7 / 9 —— 各加一句早退**

四处都是同一形状，写在各自的 `phase_enter` **之后**（护栏必须先推进）：

- `crates/stg-core/src/world/transform.rs` 的 `run_transforms`：
```rust
        self.phase_enter(super::PH_XFORM);
        if self.scene_frozen() {
            return; // C 组：xform 游标随场景冻结
        }
```
- `crates/stg-core/src/world/collide.rs` 的 `collide`：
```rust
        self.phase_enter(super::PH_COLLIDE);
        // spec §4：相位 6/7 的门禁挂在 **C 组是否在跑**，不是"是否冻结"。伤害的来源是
        // 敌方弹幕的运动 —— 玩家技能冻 C ⇒ 不跑（绝对安全窗）；ECL 演出 C 跑 ⇒ 照跑
        // （定住你、弹幕照来，演出的威胁正在于此）。**别改成 `if frozen`**。
        if self.scene_frozen() {
            return;
        }
```
- `crates/stg-core/src/world/settle.rs` 的 `settle`：同上（注释写「同 collide，spec §4；符卡计时住本相位尾 ⇒ 随之冻结，顺手堵上"用时停白嫖 survival 卡"」）。
- `crates/stg-core/src/world/cleanup.rs` 的 `cleanup`：
```rust
        self.phase_enter(super::PH_CLEANUP);
        if self.scene_frozen() {
            return; // C 组：冻 C 时没有新的越界/消弹标记产生，无需回收
        }
```

- [ ] **Step 7: 相位 10 —— 背景经过时间**

`crates/stg-core/src/world.rs` 的 `advance()`：

```rust
    pub(crate) fn advance(&mut self) {
        self.phase_enter(PH_ADVANCE);
        // 背景停滞**不是"什么都不做"就有的**：背景动画由 `frame − bg_phase_frame`
        // 驱动，而 frame 恒增 ⇒ 只冻别的会让背景照样走。冻 C 时同步推进锚点，
        // 让"背景经过的时间"不增长（spec §5）。
        if self.scene_frozen() {
            self.bg_phase_frame = self.bg_phase_frame.wrapping_add(1);
        }
        self.frame += 1;
    }
```

- [ ] **Step 8: 跑测试确认通过**

Run: `cargo test -p stg-core --lib freeze 2>&1 | tail -12`
Expected: 全部 PASS（含主测）。

Run: `cargo test --workspace 2>&1 | grep -E "^test result"`
Expected: 全绿。若有既有测试红，**先读它在断言什么**——相位拆分若改了行为就是 Step 4 写错了，不要改测试迁就。

- [ ] **Step 9: 提交**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --release --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(core): 时停的相位门禁——A/B/C 三组分区接线

门禁写在相位函数体内、phase_enter 之后：跳过相位函数会让 debug 的
PhaseGuard 断言当场失败，而且 §3.5 宪法顺序归组装层（P2）。step.rs 只
改一处——相位 2 的 run_tasks 不是 WorldBody 方法，要在组装层包。

相位 6/7 的门禁挂在 C 组是否在跑、不是'是否冻结'：照搬会让 ECL 演出
定住玩家却又免伤、演出毫无威胁。一条规则覆盖三种组合，并顺手堵上'用
时停白嫖 survival 卡'（符卡计时住相位 7 尾）。

主测用整块 World 校验和押运整张裁定表：将来谁加新计时器忘了裁定，它自动
照出来。"
```

---

### Task 4: 时停的自机入口（输入位 + 资源 + 触发）

**Files:**
- Modify: `crates/stg-core/src/input.rs`（`define_actions!` 加位 + 词表指纹测试）
- Modify: `crates/stg-core/src/player.rs`（`PlayerState.time_stops` + `Loadout.time_stops` + `TIMESTOP_FRAMES`）
- Modify: `crates/stg-core/src/step.rs`（`new_game_at` 写 `time_stops`；两条哨兵）
- Modify: `crates/stg-core/src/consts.rs`（C14 ① 段注入 `TIMESTOP_FRAMES`）
- Modify: `crates/stg-core/src/world/player.rs`（`try_time_stop`）
- Modify: `crates/stg-godot/src/bridge.rs:145`（`Loadout` 穷尽初始化会被打断）
- Test: `crates/stg-core/src/world/player.rs` 的 `mod tests`

**Interfaces:**
- Consumes: `WorldBody.freeze_left`（Task 1）；`update_players` 的 A 组分支（Task 3）。
- Produces: `crate::input::BTN_TIMESTOP: u32`；`crate::player::TIMESTOP_FRAMES: u16`；`PlayerState.time_stops: u8`；`Loadout.time_stops: u8`；`WorldBody::try_time_stop(&mut self, i: usize)`（私有）。

- [ ] **Step 1: 写失败测试**

加到 `crates/stg-core/src/world/player.rs` 的 `mod tests`：

```rust
fn press(w: &mut crate::step::World, buttons: u32) {
    let mut input = crate::input::InputFrame::empty(w.frame());
    input.actions[0].buttons = buttons;
    crate::step::step(
        w,
        &crate::tables::TABLES_V0,
        &crate::ecl::image::EclImage::empty(),
        &input,
    );
}

/// 门禁四条 + 效果。判别力：逐条断言"扣了资源"与"冻了世界"两件，只断其一的话
/// "扣费但没生效"或"生效但没扣费"各能溜过一条。
#[test]
fn time_stop_triggers_and_charges_one_use() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].time_stops = 2;
    press(&mut w, crate::input::BTN_TIMESTOP);
    assert_eq!(w.body.players[0].time_stops, 1, "应扣一次资源");
    assert_eq!(
        w.body.freeze_left[0],
        crate::player::TIMESTOP_FRAMES,
        "应写玩家技能倒计时（相位 3 写、当帧相位 4 起即冻）"
    );
    assert_eq!(w.body.freeze_left[1], 0, "不得碰 ECL 演出那一格");
}

/// 时停期间再按 = no-op **且不扣资源**（裁定 #6）。
#[test]
fn time_stop_reentry_is_free_noop() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].time_stops = 2;
    press(&mut w, crate::input::BTN_TIMESTOP);
    let left = w.body.freeze_left[0];
    press(&mut w, crate::input::BTN_TIMESTOP);
    assert_eq!(w.body.players[0].time_stops, 1, "时停中再按不得扣资源");
    assert!(w.body.freeze_left[0] < left, "也不得刷新倒计时（覆盖是 ECL 侧的语义）");
}

/// 资源为 0 时按无效。
#[test]
fn time_stop_without_charges_does_nothing() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].time_stops = 0;
    press(&mut w, crate::input::BTN_TIMESTOP);
    assert_eq!(w.body.freeze_left[0], 0);
}

/// A 组被冻（ECL 演出进行中）时不能发动时停——"你被定住了当然不能用"，
/// 这条不是特例，是 A 组门禁自动给的。
#[test]
fn time_stop_is_unavailable_while_the_actor_is_frozen() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].time_stops = 2;
    w.body.freeze_left = [0, 10];
    press(&mut w, crate::input::BTN_TIMESTOP);
    assert_eq!(w.body.freeze_left[0], 0, "被定住期间不得发动");
    assert_eq!(w.body.players[0].time_stops, 2, "也不得扣资源");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib time_stop 2>&1 | tail -10`
Expected: 编译失败——`BTN_TIMESTOP` / `time_stops` / `TIMESTOP_FRAMES` 未定义。

- [ ] **Step 3: 加输入位**

`crates/stg-core/src/input.rs` 的 `define_actions!`，在 `BTN_SLOW = 6, Level;` 之后加：

```rust
    /// 时间停止（沿触发；消费者：`world/player.rs::try_time_stop`）。
    /// **位 = 0 必须等价旧行为**——旧回放该位恒 0，故加位不是回放格式的破坏性变更。
    BTN_TIMESTOP = 7, Edge;
```

- [ ] **Step 4: 更新词表指纹**

Run: `cargo test -p stg-core --lib actions_vocab 2>&1 | grep -E "left:|right:"`
把 `crates/stg-core/src/input.rs` 里那句 `assert_eq!(actions_vocab_hash(), 0x87A3_D673_0CCB_3796);` 的值换成**实测的新值**。
同一测试模块里断言"最高位 = 6（BTN_SLOW）"的那条容量哨兵改成 **7（BTN_TIMESTOP）**，`EDGE_MASK` 的断言改成 `BTN_BOMB | BTN_TIMESTOP`。
**这是词表设计出来就要做的动作**（"词表变更须有意识地更新此值"），不是意外。

- [ ] **Step 5: 加资源字段与常量**

`crates/stg-core/src/player.rs`：

```rust
/// 时间停止的固定时长（帧）。**引擎常量而非表数据**——它不在任何表里；bomb 的数字
/// 相反，全部住 `CharacterCfg.bomb`（避免第二真相源）。若将来"每个自机时停时长不同"
/// 成为内容需求，迁进 `CharacterCfg` 的路径与 `BombCfg` 完全同构（spec §14）。
pub const TIMESTOP_FRAMES: u16 = 180;
```

`PlayerState` 加字段（放在 `bombs` 之后，与其余资源为邻）：
```rust
    /// 时间停止的剩余次数（自机能力刀）。
    pub time_stops: u8,
```
`PlayerState::spawn` 的**穷尽初始化**里加 `time_stops: ld.time_stops,`。

`Loadout` 加 `pub time_stops: u8,`，其 `Default` 给 `time_stops: 1`。

⚠️ **`Loadout` 有四个构造点，其中三处是穷尽初始化、加字段会直接编译失败**（已核实）：

| 位置 | 现状 | 处置 |
|---|---|---|
| `crates/stg-godot/src/bridge.rs:145` | 穷尽 | 加 `..Default::default()` |
| `crates/stg-core/src/step.rs:2556` | 穷尽（测试） | 加 `..Default::default()` |
| `crates/stg-core/src/step.rs:2584` | 穷尽（测试） | 加 `..Default::default()` |
| `crates/stg-godot/src/boot.rs:95` | 已有 `..Default::default()` | 不动 |

**不要给桥面加 `time_stops` 入参**——默认值 1 已够，加参数是没人要的接口扩张（裁定 R-2）。
`stg-godot` 编译不过的话两个冒烟都跑不了，所以这一步不能跳。

`crates/stg-core/src/step.rs` 的 `new_game_at`，在 `p.bombs = loadout.bombs;` 之后加：
```rust
        p.time_stops = loadout.time_stops;
```

- [ ] **Step 6: C14 ① 段注入**

`crates/stg-core/src/consts.rs` 的 `engine_consts! { structural { ... } }` 里加一行（族内顺延，别乱序）：
```rust
        TIMESTOP_FRAMES:     u16 as int = crate::player::TIMESTOP_FRAMES;
```

- [ ] **Step 7: 写触发**

`crates/stg-core/src/world/player.rs`，加私有方法并在 A 组分支里调用（**放在 `move_player` 之前**，让触发当帧就冻住相位 4 起的世界）：

```rust
    /// 时停触发（A 组）。门禁四条：动作位沿 + 资源 > 0 + 该能力未在进行 + 自机 ALIVE。
    /// 时停期间再按 = **no-op 且不扣资源**（裁定 #6）。
    fn try_time_stop(&mut self, i: usize) {
        if self.players[i].input & crate::input::BTN_TIMESTOP == 0
            || self.players[i].time_stops == 0
            || self.freeze_left[0] != 0
            || self.players[i].life_state != LIFE_ALIVE
        {
            return;
        }
        self.players[i].time_stops -= 1;
        self.freeze_left[0] = crate::player::TIMESTOP_FRAMES;
    }
```

在 Task 3 写的 A 组分支里，`self.move_player(i, tables);` **之前**插一句 `self.try_time_stop(i);`。

⚠️ **`BTN_TIMESTOP` 是 Edge 语义**：确认 `decode_input` 对 `EDGE_MASK` 的位做了沿处理，`players[i].input` 里该位只在按下那一帧为 1。若不是，就在 `try_time_stop` 里比对上一帧输入——**先读 `decode_input` 的实现再决定**，不要假设。

- [ ] **Step 8: 跑测试确认通过 + 更新哨兵**

Run: `cargo test -p stg-core --lib time_stop 2>&1 | tail -10` → 四条 PASS。
Run: `cargo test -p stg-core --lib sentinel 2>&1 | grep -E "left:|right:"` → `world_size_sentinel` 红，按实测值更新，并在它的注释里补一段：`PlayerState` 加 `time_stops: u8`，逐槽 +1 B ×2 名自机，① 走 derive、无手写清单，②③④ 同 Task 1。

- [ ] **Step 9: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep -E "^test result"
git add -A
git commit -m "feat(core): 时停的自机入口——BTN_TIMESTOP + time_stops 资源 + 相位 3 触发"
```

---

### Task 5: `513 add_time_stops` syscall

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（号常量 + 白名单 + 派发臂 + 冻结号表 + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（表层内建）
- Modify: `docs/ecl-ops.md`（号表 5xx 族）
- Regenerate: `docs/ecl-lang/7-reference.md` + `editors/vscode/stg-ecl/ecl-meta.json`

**Interfaces:**
- Consumes: `PlayerState.time_stops`（Task 4）。
- Produces: `crate::ecl::syscall::SYS_ADD_TIME_STOPS: u16 = 513`；表层内建 `add_time_stops(delta)`。

- [ ] **Step 1: 写失败测试**

加到 `crates/stg-core/src/ecl/syscall.rs` 的 `mod tests`：

```rust
/// 语义照抄 510/511/512：允许负、饱和加、钳 [0, u8::MAX]。判别力=三点（正/负/溢出），
/// 只测正数的话裸 `+`（debug 下 255+i32::MAX 会 panic）也能过。
#[test]
fn sys_add_time_stops_saturates_like_its_siblings() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    w.body.players[0].time_stops = 1;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_TIME_STOPS, &[2]).is_ok());
    assert_eq!(w.body.players[0].time_stops, 3);
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_TIME_STOPS, &[-9]).is_ok());
    assert_eq!(w.body.players[0].time_stops, 0, "扣穿停在 0，不回绕");
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_TIME_STOPS, &[i32::MAX]).is_ok());
    assert_eq!(w.body.players[0].time_stops, u8::MAX, "上钳 u8::MAX，不 panic");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib add_time_stops 2>&1 | tail -6`
Expected: `SYS_ADD_TIME_STOPS` 未定义。

- [ ] **Step 3: 实现**

`crates/stg-core/src/ecl/syscall.rs`，号常量紧跟 `SYS_ADD_POWER` 之后：
```rust
/// 时停次数增量（自机能力刀）。语义同 [`SYS_ADD_LIVES`]，钳 `[0, u8::MAX]`。
pub const SYS_ADD_TIME_STOPS: u16 = 513;
```
`syscall_implemented` 的白名单里，`| SYS_ADD_LIVES` 那一串加 `| SYS_ADD_TIME_STOPS`。
派发臂紧跟 `SYS_ADD_POWER` 之后：
```rust
        SYS_ADD_TIME_STOPS => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            p.time_stops = (p.time_stops as i32).saturating_add(d).clamp(0, u8::MAX as i32) as u8;
            Ok(())
        }
```
`frozen_table()` 里紧跟 `(SYS_ADD_POWER, "add_power", 5)` 之后加 `(SYS_ADD_TIME_STOPS, "add_time_stops", 5),`。

- [ ] **Step 4: 表层内建**

`crates/stg-ecl-compiler/src/lang/builtins.rs`，紧跟 `add_power` 那条之后：
```rust
    Builtin {
        name: "add_time_stops",
        syscall: syscall::SYS_ADD_TIME_STOPS,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "时停次数增量;同 add_lives 语义(允许负、饱和加、钳 [0,255])",
        param_names: &["delta"],
    },
```

- [ ] **Step 5: 号表文档 + 重生成元数据**

`docs/ecl-ops.md` 的 5xx 族，在 512 那行之后加：
```
| 513 | `add_time_stops`（自机能力刀） | delta | —（同 510，写 `time_stops`，钳 `[0, u8::MAX]`） |
```

```bash
cargo run -q -p stg-harness -- gen-ecl-meta
```

- [ ] **Step 6: 跑测试确认通过并提交**

Run: `cargo test --workspace 2>&1 | grep -E "^test result"`
Expected: 全绿（号表冻结测试、白名单一致性测试、`ecl-ops.md` 号表扫描测试都会验这条）。

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && git commit -m "feat(ecl): 513 add_time_stops——时停次数的补给口"
```

---

### Task 6: `560 time_stop_player` syscall

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`
- Modify: `docs/ecl-ops.md`
- Regenerate: `docs/ecl-lang/7-reference.md` + `editors/vscode/stg-ecl/ecl-meta.json`

**Interfaces:**
- Consumes: `WorldBody.freeze_left`（Task 1）。
- Produces: `crate::ecl::syscall::SYS_TIME_STOP_PLAYER: u16 = 560`；表层内建 `time_stop_player(frames)`。

- [ ] **Step 1: 写失败测试**

```rust
/// 写 freeze_left[1]（冻 A+B），不碰 [0]。
#[test]
fn sys_time_stop_player_writes_the_cutscene_slot() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[90]).is_ok());
    assert_eq!(w.body.freeze_left, [0, 90]);
}

/// 重入取**覆盖**（后写为准），不取最大、不叠加：脚本是权威，覆盖可预测。
/// 判别力：先写大值再写小值——取最大或叠加的实现在这里会给出 ≥ 90 的值。
#[test]
fn sys_time_stop_player_reentry_overwrites() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[90]).is_ok());
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[10]).is_ok());
    assert_eq!(w.body.freeze_left[1], 10, "后写为准");
}

/// `time_stop_player(0)` = **立即解除**，天然的取消 API（不另开 syscall）。
#[test]
fn sys_time_stop_player_zero_cancels() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    w.body.freeze_left[1] = 50;
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[0]).is_ok());
    assert_eq!(w.body.freeze_left[1], 0);
}

/// 参数收窄照 D19 判例：`u16::try_from` 失败 ⇒ P4-b **整条 no-op** + contract_viol
/// + BAD_ARGS，**不钳位、不 Fault**。判别力：负值与 65536 各一腿——钳位实现会给出
/// 0 或 65535 而非"原值不动"。
#[test]
fn sys_time_stop_player_rejects_out_of_range() {
    for bad in [-1i32, 65536] {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.freeze_left[1] = 7;
        let cv0 = w.body.diag.contract_viol;
        assert!(
            call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[bad]).is_ok(),
            "P4-b 是安全结果不是 Fault"
        );
        assert_eq!(w.body.freeze_left[1], 7, "越界 {bad} 必须整条 no-op（不钳位）");
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "越界 {bad} 须计一次违约");
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib time_stop_player 2>&1 | tail -6`
Expected: `SYS_TIME_STOP_PLAYER` 未定义。

- [ ] **Step 3: 实现**

号常量（5xx 族开 56x 小组，**新号落族内、永不乱序追加**）：
```rust
/// 停住**自机**的时间（自机能力刀，ECL 演出方向）：写 `freeze_left[1]` ⇒ 冻 A+B
/// （自机不能移动/发新弹，已在场上的自机弹也冻住），敌方照常行动。
/// `frames = 0` 即**立即解除**；重入取覆盖（后写为准）。
pub const SYS_TIME_STOP_PLAYER: u16 = 560;
```
白名单加 `| SYS_TIME_STOP_PLAYER`。派发臂：
```rust
        SYS_TIME_STOP_PLAYER => {
            let n = pop(task)?;
            // D19 判例：越界 → P4-b 整条 no-op + 计数 + BAD_ARGS，不钳位、不 Fault。
            let Ok(frames) = u16::try_from(n) else {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                return Ok(());
            };
            ctx.body.freeze_left[1] = frames;
            Ok(())
        }
```
`frozen_table()` 加 `(SYS_TIME_STOP_PLAYER, "time_stop_player", 5),`。

⚠️ `freeze_left` 与 `diag`/`last_status` 都是 `pub(crate)`，`syscall.rs` 在同 crate 内可直接写。若 `last_status` 有专用写口（如 `set_status`），**用那个口**，别绕过——先读 `world.rs` 里同族 syscall 怎么写的。

- [ ] **Step 4: 表层内建 + 文档 + 重生成**

`builtins.rs`：
```rust
    Builtin {
        name: "time_stop_player",
        syscall: syscall::SYS_TIME_STOP_PLAYER,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "停住自机的时间 frames 帧(自机不能动/不能发新弹,自机弹也冻住;敌方照跑);0 = 立即解除;重入覆盖;越界 no-op+计数",
        param_names: &["frames"],
    },
```
`docs/ecl-ops.md` 的 5xx 族末尾加一行 560 条目（照 §8.2 的三条口径写全）。
```bash
cargo run -q -p stg-harness -- gen-ecl-meta
```

- [ ] **Step 5: 手册补一节**

`docs/ecl-lang/6-spell-and-stage.md` 的「场面与账面写操作」加一节 `### time_stop_player() —— 定住自机`，写清：冻 A+B（不能移动/不能发新弹/自机弹也停）、**碰撞照跑所以弹幕仍会打死你**（这是演出的威胁所在）、`0` 立即解除、重入覆盖、越界 no-op。

- [ ] **Step 6: 跑测试并提交**

```bash
cargo test --workspace 2>&1 | grep -E "^test result"
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && git commit -m "feat(ecl): 560 time_stop_player——ECL 演出方向的时停入口"
```

---

### Task 7: `BombCfg` 表结构与 v0 内容

**Files:**
- Modify: `crates/stg-core/src/tables.rs`（`BombCfg`/`BombField`/`BombOrigin` + `CharacterCfg.bomb` + `build_tables_v0` + `validate` + **`to_bytes`/`from_bytes` 两侧**）
- Regenerate & commit: `crates/stg-core/src/tables/tables_v0.bin`
- Test: `crates/stg-core/src/tables.rs` 的 `mod tests`

⚠️ **先读这条，它决定本 Task 的真实范围（裁定 R-3，已在代码里核实）**：
`TABLES_V0` **不是** `build_tables_v0()` 直接来的，而是
`from_bytes(include_bytes!("tables/tables_v0.bin"))`（`tables.rs:282`）——**内建表来自提交
进仓的二进制**。所以给 `CharacterCfg` 加字段必须连表的 wire format 一起改，否则：
① `from_bytes` 里的 `CharacterCfg { .. }` 构造点（`tables.rs:634` 附近）**直接编译失败**；
② 就算补上默认值，内建表也拿不到 bomb 配置，"表驱动"名存实亡。

**Interfaces:**
- Consumes: `crate::field::{FIELD_CLEAR_BULLETS, FIELD_DAMAGE, FIELD_RADIUS_FULLSCREEN}`。
- Produces: `crate::tables::BombCfg { frames: u16, invuln: u16, attract_items: bool, fields: Box<[BombField]> }` · `BombField { origin: BombOrigin, radius: Fx, flags: u8, dmg_per_frame: u16, life: u16 }` · `BombOrigin { FieldCenter, PlayerAtCast }` · `CharacterCfg.bomb: BombCfg`。

- [ ] **Step 1: 写失败测试**

```rust
/// v0 内容锚点：两条 field（全屏消弹 + 起爆点伤害圆），120 帧，吸道具。
/// 改内容要有意识地改本测试。
#[test]
fn bomb_cfg_v0_is_two_fields() {
    let b = &TABLES_V0.characters[0].bomb;
    assert_eq!((b.frames, b.invuln, b.attract_items), (120, 120, true));
    assert_eq!(b.fields.len(), 2);
    assert_eq!(b.fields[0].origin, BombOrigin::FieldCenter);
    assert_eq!(b.fields[0].flags, crate::field::FIELD_CLEAR_BULLETS);
    assert_eq!(b.fields[1].origin, BombOrigin::PlayerAtCast);
    assert_eq!(b.fields[1].flags, crate::field::FIELD_DAMAGE);
    assert_eq!(b.fields[1].dmg_per_frame, 4);
    // 两条都活满整段 —— 消弹 field 的 life = frames 是"持续保护"的来源（spec §10.4）
    assert!(b.fields.iter().all(|f| f.life == b.frames));
}

/// validate 的四条：radius 越界 / flags 含未定义位 / frames==0 / life==0 都要被拒。
/// 判别力=四条各造一个坏表，只测一条的话另外三条的校验漏写也绿。
///
/// ⚠️ `WorldTables::validate()` 返回 **`bool`**（不是 `Result`）——照本文件既有口径。
#[test]
fn bomb_cfg_validate_rejects_bad_rows() {
    let mk = |mutate: &dyn Fn(&mut BombCfg)| {
        let mut t = build_tables_v0();
        let mut b = t.characters[0].bomb.clone();
        mutate(&mut b);
        t.characters[0].bomb = b;
        t
    };
    assert!(!mk(&|b| b.fields[0].radius = Fx::from_int(-1)).validate(), "radius 越界须拒");
    assert!(!mk(&|b| b.fields[0].flags = 0x80).validate(), "未定义 flags 位须拒");
    assert!(!mk(&|b| b.frames = 0).validate(), "frames==0 须拒");
    assert!(!mk(&|b| b.fields[0].life = 0).validate(), "life==0 须拒");
    assert!(build_tables_v0().validate(), "内建表本身必须合法");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib bomb_cfg 2>&1 | tail -6`
Expected: `BombCfg` 未定义。

- [ ] **Step 3: 加类型**

`crates/stg-core/src/tables.rs`，紧跟 `ShotTypeCfg` 之后：

```rust
/// 一发 bomb 的完整描述（**静态数据**，住 `WorldTables`、不进 `World`——与 `ShotTypeCfg`
/// 同构，M0-17 立下的先例）。把"铺哪些 field / 多久 / 吸不吸道具"做成数据而非代码，是
/// 为了让将来的 bomb 变体成为**换表**而不是改引擎；这不违反 P5（数据不是回调）。
///
/// ⚠️ **数据变不出新形状**：`FieldPool` 只有圆。"锁定敌人的 bomb"只需加一个
/// [`BombOrigin`] 变体（跟随 = 上层每帧重铺 `life = 1`，是 `FieldPool` 设计时就写好的
/// 用法）；但"激光形状的 bomb"必须给 field 加形状字段并改碰撞行 6/7 —— 那是**改碰撞
/// 矩阵**，过评审、另开一刀（spec §13）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BombCfg {
    /// 效果时长（帧）。
    pub frames: u16,
    /// 无敌帧。**允许 > `frames`**：那正是"防炸完立刻死"的旋钮，调它不改任何结构。
    pub invuln: u16,
    /// 起爆当帧是否全屏吸道具。
    pub attract_items: bool,
    /// 起爆时铺的作用区，**按声明序**（I4）。
    pub fields: Box<[BombField]>,
}

/// bomb 铺的一条作用区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BombField {
    pub origin: BombOrigin,
    pub radius: Fx,
    /// `FIELD_CLEAR_BULLETS` | `FIELD_DAMAGE` 的组合。
    pub flags: u8,
    pub dmg_per_frame: u16,
    pub life: u16,
}

/// 作用区圆心的来源。**用枚举而非 bool**，并在起爆处以穷尽 `match` 消费：加变体而忘了
/// 处理 ⇒ **编译不过**（D18 立下的押运手法）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BombOrigin {
    /// 场心（全屏效果用）。
    FieldCenter,
    /// **起爆那一帧**的自机位置，之后不动（裁定 #10：不跟随）。
    PlayerAtCast,
}
```

`CharacterCfg` 加字段 `pub bomb: BombCfg,`。

⚠️ `CharacterCfg` 现在含 `Box`，若它派生了 `Copy` 会编译失败——`ShotTypeCfg` 已经是 `Box` 且"去 `Copy`、留 `Clone`"，照它办。

- [ ] **Step 4: 填 v0 内容**

`build_tables_v0()` 的 `CharacterCfg { ... }` 里加：

```rust
            bomb: BombCfg {
                frames: 120,
                invuln: 120,
                attract_items: true,
                fields: Box::new([
                    // ① 全屏消弹：life = frames ⇒ 整段期间**逐帧消掉新飞进来的弹**，
                    //    bomb 的"保护时长"因此天然成立（spec §10.4）。
                    BombField {
                        origin: BombOrigin::FieldCenter,
                        radius: crate::field::FIELD_RADIUS_FULLSCREEN,
                        flags: crate::field::FIELD_CLEAR_BULLETS,
                        dmg_per_frame: 0,
                        life: 120,
                    },
                    // ② 起爆点伤害圆（不跟随）：120 帧 × 4 ≈ 480 伤害，约半管风铃卡血。
                    BombField {
                        origin: BombOrigin::PlayerAtCast,
                        radius: Fx::from_int(120),
                        flags: crate::field::FIELD_DAMAGE,
                        dmg_per_frame: 4,
                        life: 120,
                    },
                ]),
            },
```

- [ ] **Step 5: 加 `validate`**

`WorldTables::validate()` 返回 **`bool`**（`crates/stg-core/src/tables.rs:291`），坏表一律
`return false`——**不要新造 `Result`**。在既有的 `for c in &self.characters { ... }` 循环体内、
`shot` 校验之后追加：

```rust
            // bomb 描述层校验（自机能力刀）。半径用与其余池同一把尺 `radius_in_range`
            // （`create_field` 也会双边钳，但表校验先拦更响亮）。
            if c.bomb.frames == 0 {
                return false;
            }
            for f in c.bomb.fields.iter() {
                if f.life == 0 {
                    return false;
                }
                if f.flags
                    & !(crate::field::FIELD_CLEAR_BULLETS | crate::field::FIELD_DAMAGE)
                    != 0
                {
                    return false;
                }
                if !radius_in_range(f.radius) {
                    return false;
                }
            }
```

⚠️ `radius_in_range` 是本文件既有的私有助手（`appearances` 与角色半径都用它），直接复用，
**别自己写一遍双边比较**。

- [ ] **Step 6: 扩展表二进制格式（`to_bytes` / `from_bytes` 两侧）**

`to_bytes()`（`tables.rs:459`）的 characters 循环里，在 `option_pos` 之后追加 bomb 段。
**写入顺序 = 读出顺序**，两侧必须逐字对应：

```rust
            // bomb 段（自机能力刀）：frames/invuln/attract_items + 变长 fields。
            out.extend_from_slice(&c.bomb.frames.to_le_bytes());
            out.extend_from_slice(&c.bomb.invuln.to_le_bytes());
            out.push(c.bomb.attract_items as u8);
            out.extend_from_slice(&(c.bomb.fields.len() as u32).to_le_bytes());
            for f in c.bomb.fields.iter() {
                out.push(match f.origin {
                    BombOrigin::FieldCenter => 0u8,
                    BombOrigin::PlayerAtCast => 1u8,
                });
                out.extend_from_slice(&f.radius.raw().to_le_bytes());
                out.push(f.flags);
                out.extend_from_slice(&f.dmg_per_frame.to_le_bytes());
                out.extend_from_slice(&f.life.to_le_bytes());
            }
```

`from_bytes()`（`tables.rs:526`）的对应位置读回来。**枚举的反序列化必须拒绝未知判别值**
（坏字节不得变成"默认值"——那是静默数据损坏）：

```rust
        let bomb_frames = r.u16()?;
        let bomb_invuln = r.u16()?;
        let bomb_attract = r.u8()? != 0;
        let nbf = r.u32()? as usize;
        let mut bomb_fields = Vec::with_capacity(nbf);
        for _ in 0..nbf {
            let origin = match r.u8()? {
                0 => BombOrigin::FieldCenter,
                1 => BombOrigin::PlayerAtCast,
                _ => return Err(TableLoadError::…), // 照本文件既有的坏数据错误变体
            };
            bomb_fields.push(BombField {
                origin,
                radius: r.fx()?,
                flags: r.u8()?,
                dmg_per_frame: r.u16()?,
                life: r.u16()?,
            });
        }
```

并在 `CharacterCfg { .. }` 构造里加 `bomb: BombCfg { frames: bomb_frames, invuln:
bomb_invuln, attract_items: bomb_attract, fields: bomb_fields.into_boxed_slice() }`。

⚠️ **`r.u8()` / `r.u16()` 若不存在就照 `r.fx()`/`r.u32()` 的形状加**（同一个 `Reader`
辅助结构，别另起一套）。错误变体**照本文件既有的 `TableLoadError` 用**，不要新造。

- [ ] **Step 7: 重烘 `tables_v0.bin` 并复验**

```bash
cargo run -q -p stg-harness -- bake-tables
cargo run -q -p stg-harness -- verify-tables
git status --short crates/stg-core/src/tables/tables_v0.bin
```
Expected: `bake-tables` 重写那个 `.bin`（文件出现在 `git status` 里）、`verify-tables`
通过。**必须把新的 `.bin` 一起提交**——它是内建表的唯一真相源。

⚠️ 若 `bake-tables` 不负责 `tables_v0.bin`（它主要烘 `math/tables/`），改用 harness 的
`crates/stg-harness/src/tables.rs:90` 那条 `gen_world_tables_v0` 走的入口；**先读那个文件
确认哪个子命令写它**，别猜。

- [ ] **Step 8: 加一条往返测试**

```rust
/// bomb 段的 to_bytes/from_bytes 往返：写出去再读回来必须逐字段相等。
/// 判别力：漏写任何一个字段、或读写顺序错位，这条都会红（而只测"能解析"的写法不会）。
#[test]
fn bomb_cfg_survives_a_bytes_roundtrip() {
    let t0 = build_tables_v0();
    let t1 = WorldTables::from_bytes(&t0.to_bytes()).expect("往返应成功");
    assert_eq!(t1.characters[0].bomb, t0.characters[0].bomb);
}

/// 坏的 origin 判别值必须被**拒绝**，不得静默变成默认值（静默 = 数据损坏）。
#[test]
fn bomb_origin_rejects_unknown_discriminant() {
    let mut bytes = build_tables_v0().to_bytes();
    // 找到 bomb 段第一条 field 的 origin 字节并改成非法值 —— 用 from_bytes 的
    // 错误类型断言被拒。定位方法：先跑通往返测试，再用二分或按写入顺序算偏移。
    let pos = bytes
        .windows(1)
        .position(|_| false)
        .unwrap_or(0);
    let _ = (pos, &mut bytes);
    // 实现时把上面两句换成真实定位；断言形如：
    // assert!(WorldTables::from_bytes(&bytes).is_err(), "未知 origin 判别值须被拒");
}
```

⚠️ 第二条测试的定位方式**由实现者决定**（按写入顺序算偏移最稳）。**不许留成空壳**——
要么写成真断言，要么删掉它并在报告里说明为什么无法定位。

- [ ] **Step 9: 跑测试并提交**

```bash
cargo test -p stg-core --lib bomb_cfg 2>&1 | tail -6
cargo test --workspace 2>&1 | grep -E "^test result"
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && git commit -m "feat(core): BombCfg 表结构 + v0 内容（表驱动的 bomb 描述层）"
```

---

### Task 8: bomb 触发、状态机与 deathbomb

**Files:**
- Modify: `crates/stg-core/src/world/player.rs`（`try_bomb` + `bomb_timer` 倒数）
- Test: `crates/stg-core/src/world/player.rs` 的 `mod tests`

**Interfaces:**
- Consumes: `BombCfg`（Task 7）；`update_players` 的 A/C 分支（Task 3）；`WorldBody::create_field` / `attract_all_items`（既有）。
- Produces: `WorldBody::try_bomb(&mut self, i: usize, tables: &WorldTables)`（私有）。

- [ ] **Step 1: 写失败测试（五条判别腿）**

```rust
/// ①②成对：窗口内能救、窗口外救不了。**只写①的话"任何时候 bomb 都能救"照样绿。**
#[test]
fn deathbomb_inside_the_window_revives_without_costing_a_life() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].bombs = 1;
    let lives0 = w.body.players[0].lives;
    w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
    w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
    press(&mut w, crate::input::BTN_BOMB);
    assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE, "该复活");
    assert_eq!(w.body.players[0].lives, lives0, "决死救人**不扣命**");
    assert_eq!(w.body.players[0].bombs, 0, "扣一颗 bomb");
}

#[test]
fn bomb_after_the_window_closed_cannot_undo_the_death() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].bombs = 1;
    let lives0 = w.body.players[0].lives;
    w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
    w.body.players[0].state_timer = 1;
    press(&mut w, 0); // 窗口耗尽 → commit_death
    assert_eq!(w.body.players[0].lives, lives0 - 1, "已经扣命");
    press(&mut w, crate::input::BTN_BOMB);
    assert_eq!(w.body.players[0].lives, lives0 - 1, "救不回来，命不会退");
}

/// ③ 伤害圆判别式：圈**内**敌掉血、圈**外**敌不掉血。
/// 圆心重合式的摆法测不出半径映射（CLAUDE.md 点名的 M0-7 教训）。
#[test]
fn bomb_damage_field_hits_only_enemies_inside_its_radius() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].x = Fx::ZERO;
    w.body.players[0].y = Fx::from_int(200);
    w.body.players[0].bombs = 1;
    let near = crate::world::test_support::spawn_enemy(&mut w, 0, 240, 1000); // 距 40 < 120
    let far = crate::world::test_support::spawn_enemy(&mut w, 0, 40, 1000);   // 距 160 > 120
    let (ni, fi) = (w.body.enemies.get(near).unwrap(), w.body.enemies.get(far).unwrap());
    let (nhp, fhp) = (w.body.enemies.hp[ni], w.body.enemies.hp[fi]);
    press(&mut w, crate::input::BTN_BOMB);
    press(&mut w, 0);
    assert!(w.body.enemies.hp[ni] < nhp, "圈内敌必须掉血");
    assert_eq!(w.body.enemies.hp[fi], fhp, "圈外敌不得掉血");
}

/// ④ 伤害圆**不跟随**：起爆后把自机挪走，圆心不动（裁定 #10 的后半句）。
#[test]
fn bomb_damage_field_does_not_follow_the_player() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].x = Fx::ZERO;
    w.body.players[0].y = Fx::from_int(200);
    w.body.players[0].bombs = 1;
    press(&mut w, crate::input::BTN_BOMB);
    let f = w.body.fields.iter_alive().find(|&i| {
        w.body.fields.flags[i] & crate::field::FIELD_DAMAGE != 0
    }).expect("应铺了伤害 field");
    let (fx, fy) = (w.body.fields.x[f], w.body.fields.y[f]);
    w.body.players[0].x = Fx::from_int(150); // 把自机挪走
    press(&mut w, 0);
    assert_eq!((w.body.fields.x[f], w.body.fields.y[f]), (fx, fy), "圆心必须钉在起爆点");
}

/// ⑤ 持续消弹：起爆后第 60 帧新发射的弹**也被消掉**。写成 `life = 1` 的话这条当场红，
/// 而只测起爆当帧的写法对它是瞎的（spec §10.4）。
#[test]
fn bomb_clear_field_keeps_clearing_for_its_whole_duration() {
    let mut w = crate::step::World::new(1);
    w.body.players[0].bombs = 1;
    press(&mut w, crate::input::BTN_BOMB);
    for _ in 0..59 {
        press(&mut w, 0);
    }
    crate::world::test_support::bullet_at(&mut w, 0, 200); // 第 60 帧新来的弹
    press(&mut w, 0);
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "整段期间新弹也该被消掉");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib bomb 2>&1 | tail -12`
Expected: 五条全 FAIL（`try_bomb` 还不存在，按 `BTN_BOMB` 什么也不发生）。

- [ ] **Step 3: 写触发与状态机**

`crates/stg-core/src/world/player.rs`：

```rust
    /// bomb 触发（A 组）。门禁四条与时停同构，唯一不同是第四条：这里允许
    /// `LIFE_DEATHWINDOW` —— **主动 bomb 与决死救人是同一条路径**，只是入口状态不同。
    ///
    /// **决死救人为什么不用退款**：进入决死窗口时只设 `life_state` 与 `state_timer`，
    /// `lives` 一点没动；扣命只发生在 `commit_death`，而它只在 `state_timer` 归零时才跑。
    /// 所以救人 = 把状态拨回 ALIVE + 清计时（spec §10.2）。
    fn try_bomb(&mut self, i: usize, tables: &WorldTables) {
        if self.players[i].input & crate::input::BTN_BOMB == 0
            || self.players[i].bombs == 0
            || self.players[i].bomb_phase != 0
            || !matches!(self.players[i].life_state, LIFE_ALIVE | LIFE_DEATHWINDOW)
        {
            return;
        }
        let cfg = &tables.characters[self.players[i].character_id as usize].bomb;
        self.players[i].bombs -= 1;
        self.players[i].bomb_phase = 1;
        self.players[i].bomb_timer = cfg.frames;
        if self.players[i].life_state == LIFE_DEATHWINDOW {
            self.players[i].life_state = LIFE_ALIVE;
            self.players[i].state_timer = 0;
        }
        self.players[i].invuln = cfg.invuln;
        let (px, py) = (self.players[i].x, self.players[i].y);
        for f in cfg.fields.iter() {
            // 穷尽 match：加 BombOrigin 变体而忘了处理 ⇒ 编译不过（D18 手法）。
            let (x, y) = match f.origin {
                crate::tables::BombOrigin::FieldCenter => {
                    (Fx::ZERO, Fx::from_int(crate::world::FIELD_HEIGHT / 2))
                }
                crate::tables::BombOrigin::PlayerAtCast => (px, py),
            };
            self.create_field(crate::field::FieldInit {
                x,
                y,
                radius: f.radius,
                dmg_per_frame: f.dmg_per_frame,
                life: f.life,
                owner: i as u8,
                flags: f.flags,
            });
        }
        if cfg.attract_items {
            self.attract_all_items(i);
        }
    }
```

在 Task 3 写的 **C 组**分支里（`match life_state` 之后、GAMEOVER 复查之前）加 bomb 计时：
```rust
                // bomb 计时归 C 组（与 invuln、决死窗口同属"世界对自机的裁决"）
                // ⇒ 时停期间 bomb 不流逝、不浪费无敌帧（spec §10.2）。
                if self.players[i].bomb_timer > 0 {
                    self.players[i].bomb_timer -= 1;
                    if self.players[i].bomb_timer == 0 {
                        self.players[i].bomb_phase = 0;
                    }
                }
```

在 A 组分支里，`self.try_time_stop(i);` 之后加 `self.try_bomb(i, tables);`。

⚠️ **借用检查**：`cfg` 借了 `tables`（`&WorldTables`，与 `&mut self` 不冲突），但 `cfg.fields.iter()` 期间调 `self.create_field` 需要 `&mut self` —— `tables` 是独立引用，没问题。若编译器仍报错，把 `cfg.fields` 先 `let fields = cfg.fields.clone();`**不要**——改成先把要铺的 `FieldInit` 收进一个定长小数组再铺，或直接按索引循环 `for k in 0..cfg.fields.len()`。**禁止在 `stg-core` 里为绕借用检查引入堆分配。**

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core --lib bomb 2>&1 | tail -12` → 五条 PASS。
Run: `cargo test --workspace 2>&1 | grep -E "^test result"` → 全绿。

⚠️ 若 `spell.rs` 的符卡测试开始红，**先读它**：bomb 一旦真会设 `bomb_phase`，`spell.rs` 那句"bomb 起爆 ⇒ `capture_ok = 0`"就真正生效了。那是**期望的行为**，可能需要给某条既有测试补一句说明，但不要为了让它绿而改掉资格作废逻辑。

- [ ] **Step 5: 补一条"符卡资格作废真的生效了"的测试**

```rust
/// bomb 起爆 ⇒ 符卡不予收卡。`spell.rs` 的资格轮询（`settle_spells` 步 1）一直写着
/// `bomb_phase != 0 ⇒ capture_ok = 0`，但在本刀之前**没有任何东西会设 `bomb_phase`**
/// ——这条测的是那根接线终于通了，不是符卡机构自身（故放在 player.rs 而非 spell.rs）。
#[test]
fn bombing_voids_the_spell_capture() {
    let mut w = crate::step::World::new(1);
    let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
    // 参数序同 spell.rs 测试的既有用法：(slot, boss, spell_id, hp_threshold,
    // hp_start, flags, frames)。
    assert!(w.body.spell_begin_internal(0, boss, 1, 100, 1000, 0, 300));
    assert_ne!(w.body.spells[0].capture_ok, 0, "开卡时资格应在");
    w.body.players[0].bombs = 1;
    press(&mut w, crate::input::BTN_BOMB);
    assert_ne!(w.body.players[0].bomb_phase, 0, "前提：bomb 真的起爆了");
    assert_eq!(w.body.spells[0].capture_ok, 0, "起爆后本卡不予收卡");
}
```

⚠️ **先读 `crates/stg-core/src/spell.rs` 的 `mod tests` 确认 `spell_begin_internal` 的实参序**
（写本计划时的用法是 `spell_begin_internal(0, boss, 1, 100, 1000, 0, 300)`）。签名若已变，
以代码为准，别照抄本计划的字面量。

- [ ] **Step 6: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && git commit -m "feat(core): bomb 触发 + 单阶段状态机 + deathbomb

主动 bomb 与决死救人是同一条路径，只是入口状态不同。救人不需要退款逻辑
——进入决死窗口时 lives 一点没动，扣命只发生在 commit_death。"
```

---

### Task 9: 收口——容量实测、文档、清单、闸门

**Files:**
- Modify: `docs/ecl-lang/6-spell-and-stage.md`、`docs/ecl-lang/8-errors.md`、`docs/render-contract.md`
- Modify: `docs/follow-ups.md`（删 E 组）
- Modify: `PROGRESS.md`
- Modify: `stg-world-design.md`（D10 表补 bomb 相关约束）

- [ ] **Step 1: 实测满屏 bomb 的道具峰值**

bomb 的持续消弹 × 120 帧 ⇒ 星星按 1:1 生成，**再叠加全屏吸取**。这是 F12 那条路的第二个入口。上一刀刚把道具池抬到 1024，**必须验它还够**：

```bash
cargo run -q -p stg-harness -- run godot/ecl/demo --frames 9000 --rank 3
```
看 `pool_full` 与道具峰值。demo 局现在没有 bomb 输入，所以这条只是基线；**真正的验证要在 Task 9 里给 `run` 临时喂一次 bomb 输入，或在真工程里手动放一发满屏 bomb 看 HUD**。若 `pool_full` 非零，**不要**当场调池——记一条 follow-up 并在 PROGRESS 里写明，让人类裁定（同 F12 的处置流程）。

- [ ] **Step 2: 文档**

- `docs/ecl-lang/6-spell-and-stage.md`：`clear_bullets()` 那节补一句"bomb 走的是同一套消弹区机制，`BombCfg` 描述铺哪些 field"。
- `docs/ecl-lang/8-errors.md` 表加一行：`| 时停/bomb 期间的相位跳过 | 冻 C 时相位 6/7/9 不跑 ⇒ 不判定、不结算、不回收 | spec §4 |`。
- `docs/render-contract.md`：加一小节说明表现层可读 `WorldView::freeze_left()` 画停时效果，并说明**背景相位锚点在冻 C 时同步推进**（表现层不需要自己特判）。

- [ ] **Step 3: 销 follow-ups E 组**

`docs/follow-ups.md`：**整节删除 `## E. bomb 那一刀开工前`**（三条逐条核实已兑现：bomb 救人 stub 已接、graze 与消弹的口径注释不变、`FieldPool` 首个真租户已落地）。在文件头的追记区加一段「自机能力刀（2026-09-03）销账」，写明销了 E 组、以及本刀新记了什么（若 Step 1 撞出道具池问题就记在这里）。

- [ ] **Step 4: 更新 `PROGRESS.md`**

「现在」段重写（≤10 行）：位置 = 自机能力刀落地；`ENGINE_VER` 14→15；金向量新 md5；follow-ups 条数变化；在飞 = 无；下一阶段候选恢复成 M3 等。里程碑史加**一行**。

- [ ] **Step 5: 全闸门**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --release --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep -E "^test result"
cargo run -q -p stg-harness -- verify-tables
cargo run -q -p stg-harness -- check godot/ecl/demo
for r in 0 1 2 3; do cargo run -q -p stg-harness -- run godot/ecl/demo --frames 9000 --rank $r 2>&1 | grep -E "^诊断"; done
cargo run -q --release -p stg-harness -- storm
cargo run -q -p stg-harness -- golden --out /tmp/g-final.txt && md5sum /tmp/g-final.txt
bash crates/stg-godot/smoke/run-smoke.sh 2>&1 | tail -1
bash godot/smoke/run-smoke.sh 2>&1 | tail -1
```

⚠️ **`storm` 是本刀必须跑的那道闸**：`World` 变宽 ⇒ 存档 wire format 变了。
⚠️ **两个冒烟都要跑**：上一刀（道具池）就是被真工程冒烟抓住了 `playfield.gd` 的容量镜像漂移。本刀不改池 cap，但改了 `World` 布局与 `WorldView` 的读口面，仍要验。
把最终金向量 md5 填进 `PROGRESS.md`。

- [ ] **Step 6: 提交并合并**

```bash
git add -A && git commit -m "docs: 自机能力刀收口——文档、销 E 组、PROGRESS"
```
然后按 `superpowers:finishing-a-development-branch` 走合并。
