# 前置小刀(bridge-precut)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 桥刀(stg-godot)开工前还清三笔 core 侧账:A1 `ItemTypeCfg.sprite` 表列、正典 boot `World::new_game` 下沉、A2 bench 基线续表。

**Architecture:** 全部改动在 stg-core/stg-harness/docs 内,零新 crate。表列走"改 owned 构造 → 重烘焙 bin → verify-tables 对拍"既有纪律;boot 下沉是组装层(step.rs)加一个正典构造入口;bench 纯测量。

**Tech Stack:** Rust 1.92(workspace 钉死),无新依赖。

**Spec:** `docs/superpowers/specs/2026-07-24-stg-godot-bridge-design.md` §0(勘误后验收)/§2。

## Global Constraints

- **I1**:断层线以下无浮点——`sprite: u16` 是整数,合规;断层线以下**没有任何代码读它**(只有序列化/哈希碰它)。
- **表纪律**:`tables_v0.bin` 绝不手编——改 `build_tables_v0()` 后用 harness `bake-tables` 再生成、`verify-tables` 断言一致,bin 与代码**同 commit**。
- **`TABLE_VERSION` 1 → 2**(tables.rs:376),新字段序列化在 item 行**末尾追加**(attract_radius 之后)。
- **占位 sprite 值 = 类型序号**:POWER=0, POINT=1, LIFE_PIECE=2, BOMB_PIECE=3, STAR=4(spec §11.7)。
- **金向量验收(spec §0 勘误口径)**:A1 后金向量**全帧均匀漂移**(每一帧都变;部分帧变=行为回归红灯);Task 3/4 在漂移后的流上**逐字节不变**。基线文件放 `.superpowers/precut/`(未跟踪目录,勿 git add)。
- **测试判别值纪律(S1)**:序列化测试的被断言字段先灌互异非零判别值。
- 分支 `feat/bridge-precut`;commit 结尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 跑绿标准:`cargo test --workspace` + `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings`。

---

### Task 1: A1 —— `ItemTypeCfg.sprite` 表列 + 序列化 v2 + 重烘焙

**Files:**
- Modify: `crates/stg-core/src/tables.rs`(struct 33-46 区、`STD_ITEM` 101-108、`ITEM_CFG_V0` 110-131、`TABLE_VERSION` 376、`to_bytes` 395-403、`from_bytes` 479-500、tests)
- Regenerate: `crates/stg-core/src/tables/tables_v0.bin`(经 harness bake-tables,**不手编**)

**Interfaces:**
- Produces: `ItemTypeCfg` 新增 `pub sprite: u16`(Task 2 的不变性测试、桥刀渲染 join 消费);`TABLE_VERSION = 2`。

- [ ] **Step 1: 建分支 + 抓金向量基线(必须在任何代码改动之前)**

```bash
cd /data/sunyunbo/www/stg-engine
git checkout -b feat/bridge-precut
mkdir -p .superpowers/precut
cargo run --release -p stg-harness -- golden --out .superpowers/precut/golden-base.txt
wc -l .superpowers/precut/golden-base.txt   # 记住行数,Step 7 用
```

- [ ] **Step 2: 写失败测试**(tables.rs 的 `#[cfg(test)] mod tests` 内追加)

```rust
    /// A1 判别式:序列化往返保 sprite(互异非零判别值,S1 纪律)。
    #[test]
    fn item_sprite_roundtrips_with_distinct_values() {
        let mut t = build_tables_v0();
        let vals: [u16; ITEM_TYPE_COUNT] = [11, 22, 33, 44, 55];
        for (i, v) in vals.iter().enumerate() {
            t.item_cfg[i].sprite = *v;
        }
        let back = WorldTables::from_bytes(&t.to_bytes()).expect("roundtrip");
        for (i, v) in vals.iter().enumerate() {
            assert_eq!(back.item_cfg[i].sprite, *v, "row {i}");
        }
    }

    /// A1 内容健全:v0 五行 sprite 互异(占位序号 0..=4)。
    #[test]
    fn item_cfg_v0_sprites_distinct() {
        let t = build_tables_v0();
        for a in 0..ITEM_TYPE_COUNT {
            for b in (a + 1)..ITEM_TYPE_COUNT {
                assert_ne!(t.item_cfg[a].sprite, t.item_cfg[b].sprite, "{a} vs {b}");
            }
        }
    }
```

- [ ] **Step 3: 跑出编译失败**

Run: `cargo test -p stg-core item_sprite -- --nocapture 2>&1 | head -20`
Expected: 编译错误 `no field 'sprite' on type ItemTypeCfg`(TDD 红)。

- [ ] **Step 4: 最小实现**——五处改动:

(a) struct(tables.rs:39 起)加字段:

```rust
pub struct ItemTypeCfg {
    pub score: u32,
    pub eject_speed: Fx,
    pub terminal_vy: Fx,
    pub magnet_speed: Fx,
    pub pickup_radius: Fx,
    pub attract_radius: Fx,
    /// 贴图索引(A1,2026-07-24):`item_type→贴图` 的单一真相源。**断层线以下无人读**
    /// ——渲染期消费者 join(池列方案否决评审记录:spec 2026-07-24 §2.1)。
    pub sprite: u16,
}
```

(b) `STD_ITEM`(101-108)补 `sprite: 0,`(基座默认;各行显式覆写)。

(c) `ITEM_CFG_V0` 五行各显式写 sprite(勿依赖 `..STD_ITEM` 默认,互异要肉眼可见):

```rust
const ITEM_CFG_V0: [ItemTypeCfg; ITEM_TYPE_COUNT] = [
    ItemTypeCfg {
        score: 10,
        sprite: 0,
        ..STD_ITEM
    }, // POWER
    ItemTypeCfg {
        score: 100,
        sprite: 1,
        ..STD_ITEM
    }, // POINT
    ItemTypeCfg {
        score: 50,
        sprite: 2,
        ..STD_ITEM
    }, // LIFE_PIECE
    ItemTypeCfg {
        score: 50,
        sprite: 3,
        ..STD_ITEM
    }, // BOMB_PIECE
    ItemTypeCfg {
        score: 30,
        sprite: 4,
        ..STD_ITEM
    }, // STAR
];
```

(d) `to_bytes`(item 行循环,396-403)在 `attract_radius` 写出后追加:

```rust
            out.extend_from_slice(&it.sprite.to_le_bytes());
```

(e) `from_bytes`(item 行循环,488-497)在 `attract_radius: r.fx()?,` 后追加:

```rust
                sprite: r.u16()?,
```

(f) `TABLE_VERSION`(376):`const TABLE_VERSION: u16 = 2;`

- [ ] **Step 5: 立即重烘焙(在跑测试之前——旧 bin 是 version 1,不烘 TABLES_V0 会全线报 UnsupportedVersion)**

```bash
cargo run -p stg-harness -- bake-tables
cargo run -p stg-harness -- verify-tables
git status --short   # 确认 crates/stg-core/src/tables/tables_v0.bin 已变
```

Expected: `verify-tables: 全部表与 commit 字节一致 ✔`。

- [ ] **Step 6: 全量测试**

Run: `cargo test --workspace`
Expected: 全绿含两个新测试。若有既有测试写死 item 行宽/总字节数而红:如实核对后更新数字(item 行 24B→26B),**不许弱化判别式断言**;若红的原因不是行宽,停下来查,不许瞎改。

- [ ] **Step 7: 金向量均匀漂移断言**

```bash
cargo run --release -p stg-harness -- golden --out .superpowers/precut/golden-a1.txt
n=$(wc -l < .superpowers/precut/golden-base.txt)
d=$(diff .superpowers/precut/golden-base.txt .superpowers/precut/golden-a1.txt | grep -c '^<')
echo "total=$n changed=$d"
test "$n" -eq "$d" && echo UNIFORM-SHIFT-OK
```

Expected: `UNIFORM-SHIFT-OK`(每一帧都变=tables_hash 常量项漂移)。若 `d < n`(部分帧变):**真行为回归,红灯停**,回查 Step 4 是否碰了序列化以外的路径。

- [ ] **Step 8: fmt/clippy + commit(bin 与代码同 commit)**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-core/src/tables.rs crates/stg-core/src/tables/tables_v0.bin
git commit -m "feat(tables): A1 ItemTypeCfg.sprite 表列——item_type→贴图单一真相源,表 v2 重烘焙(道具渲染 join 归消费端,金向量 tables_hash 均匀漂移)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: A1 行为不变性判别式测试(harness)

**Files:**
- Modify: `crates/stg-harness/src/main.rs`(`#[cfg(test)] mod tests` 内追加;编译入口 `stg_ecl_compiler::lang::compile` 与 `step_with_director` 该文件均已在用,import 按文件现状补)

**Interfaces:**
- Consumes: Task 1 的 `ItemTypeCfg.sprite`;`stg_core::tables::build_tables_v0()`;`World::new_with_tables(seed, &tables)`;`start_main(&image)`;`step_with_director(&mut world, &tables, &image, &input, |_| {})`;`World::checksum()`。

- [ ] **Step 1: 写测试**(spec §0 勘误验收②的落地)

```rust
    /// A1 行为不变性判别式:仅 item sprite 值不同的两份 owned 表(content_hash 同为 0)
    /// → 同种子世界含道具全生命周期(掉落/下坠/磁吸/拾取)演化,逐帧校验和相等;
    /// 序列化字节则必须不同(sprite 参与身份)。
    #[test]
    fn item_sprite_values_never_affect_world_evolution() {
        const DROP_SRC: &str = r#"
async sub main() {
    loop {
        drop_item(96.0, 64.0, 0);
        drop_item(128.0, 64.0, 1);
        drop_item(160.0, 64.0, 2);
        drop_item(192.0, 64.0, 3);
        drop_item(224.0, 64.0, 4);
        wait(40);
    }
}
"#;
        let image = match stg_ecl_compiler::lang::compile(DROP_SRC, "drop.ecl") {
            Ok(i) => i,
            Err(errors) => {
                let msg: Vec<String> = errors.iter().map(|e| e.render("drop.ecl")).collect();
                panic!("drop.ecl 编译失败:\n{}", msg.join("\n\n"));
            }
        };

        let ta = stg_core::tables::build_tables_v0();
        let mut tb = stg_core::tables::build_tables_v0();
        for (i, cfg) in tb.item_cfg.iter_mut().enumerate() {
            cfg.sprite = 100 + i as u16; // 与 ta 的 0..=4 全互异
        }
        assert_ne!(ta.to_bytes(), tb.to_bytes(), "sprite 必须参与序列化身份");

        let mut wa = stg_core::step::World::new_with_tables(7, &ta);
        let mut wb = stg_core::step::World::new_with_tables(7, &tb);
        wa.start_main(&image).expect("main a");
        wb.start_main(&image).expect("main b");

        for frame in 0..300u32 {
            let mut input = InputFrame::empty(frame);
            input.actions[0].buttons = BTN_LEFT; // 自机移动,扰动拾取几何
            step_with_director(&mut wa, &ta, &image, &input, |_| {});
            step_with_director(&mut wb, &tb, &image, &input, |_| {});
            assert_eq!(
                wa.checksum(),
                wb.checksum(),
                "frame {frame} 演化分歧:sprite 值泄漏进模拟"
            );
        }
    }
```

注:`InputFrame`/`BTN_LEFT`/`step_with_director` 若测试模块尚未 import,按 `cmd_golden`(main.rs:842-861)的既有用法补 `use`。

- [ ] **Step 2: 跑过**

Run: `cargo test -p stg-harness item_sprite_values_never -- --nocapture`
Expected: PASS。若某帧 assert 炸:**A1 实现有真泄漏,红灯停**——回 Task 1 查(合法改动不可能让 owned 表的 sprite 值进演化)。

- [ ] **Step 3: fmt + commit**

```bash
cargo fmt --all
git add crates/stg-harness/src/main.rs
git commit -m "test(harness): A1 判别式——仅 sprite 值不同的两表驱动同种子世界 300 帧逐帧校验和相等(行为不变性验收,spec §0 勘误②)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: 正典 boot `World::new_game` 下沉 core

**Files:**
- Modify: `crates/stg-core/src/step.rs`(`impl World` 内,`new_with_tables` 之后加方法)
- Test: `crates/stg-core/src/ecl/binding.rs`(tests 模块——`test_image`/`root_and_async_image()` helper 都在这,`start_main` 的测试同院)

**Interfaces:**
- Consumes: `World::new(seed)`(step.rs:53)、`WorldBody::set_var(u16, i32)`(world.rs:628)、`crate::consts::GVAR_RANK: u16`、`World::start_main(&image) -> Result<u16, TaskStartError>`(binding.rs:105)。
- Produces: `World::new_game(seed: u64, rank: i32, image: &EclImage) -> Result<Box<World>, TaskStartError>`(桥刀 `boot.rs` 与一切后续消费者的唯一正典开局入口)。

- [ ] **Step 1: 写失败测试**(binding.rs tests 模块内,复用现有 `root_and_async_image()` helper)

```rust
    /// 正典开局(spec 2026-07-24 §2.3):rank 入 GVAR_RANK、main 一次性已启。
    #[test]
    fn new_game_canonical_boot() {
        let image = root_and_async_image();
        let mut w = World::new_game(42, 3, &image).expect("boot");
        assert_eq!(
            w.body.get_var(crate::consts::GVAR_RANK),
            3,
            "rank 写入正典槽"
        );
        assert!(
            matches!(
                w.start_main(&image),
                Err(TaskStartError::MainAlreadyStarted)
            ),
            "new_game 已启 main,一次性约束生效"
        );
    }

    /// 握手 §7.2"同一确定性初始化":同参两次 new_game 世界校验和全等。
    #[test]
    fn new_game_same_inputs_same_world() {
        let image = root_and_async_image();
        let wa = World::new_game(7, 2, &image).expect("a");
        let wb = World::new_game(7, 2, &image).expect("b");
        assert_eq!(wa.checksum(), wb.checksum(), "同参必同世界");
    }
```

(若 tests 模块缺 `World` 的 use,按该模块现状补。)

- [ ] **Step 2: 跑出编译失败**

Run: `cargo test -p stg-core new_game -- --nocapture 2>&1 | head -10`
Expected: `no function or associated item named 'new_game'`。

- [ ] **Step 3: 实现**(step.rs `impl World`,紧跟 `new_with_tables` 之后;`TaskStartError`/`EclImage` 的 use 路径按 binding.rs 的定义补)

```rust
    /// 正典开局(spec 2026-07-24 §2.3)——回放可移植性与联机握手 §7.2"初始状态由
    /// 双方从同一确定性初始化各自构造"的**唯一入口**:new + 写 `GVAR_RANK` +
    /// `start_main`(Stage 属主)。场景实体摆放归脚本(`spawn_enemy`/`boss_set`/
    /// `spell_begin` 均为 builtin);**编译不下沉**,只吃成品镜像(依赖方向不可反转)。
    /// rainbow 金向量的手摆 boss boot 是冻结遗产,不迁移(spec §2.3)。
    pub fn new_game(
        seed: u64,
        rank: i32,
        image: &crate::ecl::image::EclImage,
    ) -> Result<Box<World>, crate::ecl::binding::TaskStartError> {
        let mut w = World::new(seed);
        w.body.set_var(crate::consts::GVAR_RANK, rank);
        w.start_main(image)?;
        Ok(w)
    }
```

(若 `TaskStartError` 实际定义路径不同,以 `grep -rn 'enum TaskStartError' crates/stg-core/src/` 为准改 use;**不许**为路径便利搬动类型。)

- [ ] **Step 4: 跑过 + 全量**

Run: `cargo test -p stg-core new_game && cargo test --workspace`
Expected: 全绿。

- [ ] **Step 5: 金向量不变断言(在 A1 漂移后的流上逐字节不变)**

```bash
cargo run --release -p stg-harness -- golden --out .superpowers/precut/golden-t3.txt
diff .superpowers/precut/golden-a1.txt .superpowers/precut/golden-t3.txt && echo GOLDEN-UNCHANGED-OK
```

Expected: `GOLDEN-UNCHANGED-OK`(加法 API 不动演化)。

- [ ] **Step 6: fmt/clippy + commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-core/src/step.rs crates/stg-core/src/ecl/binding.rs
git commit -m "feat(step): 正典开局 World::new_game 下沉组装层——回放可移植/握手 §7.2 的同一确定性初始化唯一入口(new + GVAR_RANK + start_main,场景摆放归脚本)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: A2 bench 续表 + 簿记收口

**Files:**
- Modify: `docs/bench-baseline.md`(文末追加第三轮段)
- Modify: `docs/follow-ups.md`(删 A1/A2 两条,无墓碑纪律)
- Modify: `stg-world-design.md:535-536`(ItemTypeCfg 字段清单六→七)
- Modify: `PROGRESS.md`(史加一行 + 重写「现在」段)

- [ ] **Step 1: 跑 bench(release,默认 600 帧)**

```bash
cargo run --release -p stg-harness -- bench 2>&1 | tee .superpowers/precut/bench-2026-07.txt
```

- [ ] **Step 2: 续表**——`docs/bench-baseline.md` 文末追加一段,**数字全部抄 Step 1 实测输出,不许沿用旧值**:

```markdown
## 内存账 + step 曲线(2026-07-24 第三轮,release;表 v2/sprite 列 + M1 任务池入账后)

- **World 总计 X.XX MB**(实测值;对照第二轮 0.92 MB——M1 任务池 ~108KB/通道 B 7KB/
  符卡槽等其后新账在此轮一并入账,follow-ups A2 所记"1.03MB"于此对账)。
- (表格式与第二轮同构:场景 × 稳态弹 × step 均/p50/p99 × 快照 × 校验和,逐行抄实测。)

### 结论增补(第三轮)

- 校验和/step 比值实测 XX×(第二轮口径 10-20×,A2 所记"远超"于此对账定数);
- M2 帧预算决策依据以本轮为准。
```

(段内 `X`/表行 = 占位说明,执行时**必须**替换为实测数;交付物里不得残留 X。)

- [ ] **Step 3: 簿记三件**

- `docs/follow-ups.md`:整段删除 `### A1.` 与 `### A2.` 两条(A 组若因此空了,留组头与判据引言);
- `stg-world-design.md:535-536`:字段清单补 `sprite`,改为
  `(score/eject_speed/terminal_vy/magnet_speed/pickup_radius/attract_radius/sprite 七字段;sprite 为 A1 表列,2026-07-24,渲染 join 归消费端)`(保持原行文风格接排);
- `PROGRESS.md`:史表加一行 `2026-07-24 前置小刀(A1 表列 sprite/正典 boot new_game/bench 第三轮)`;「现在」段重写为:前置账已清,下一步桥刀(spec §3-§9,plan 另写)。

- [ ] **Step 4: 终验 + commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run --release -p stg-harness -- golden --out .superpowers/precut/golden-final.txt
diff .superpowers/precut/golden-t3.txt .superpowers/precut/golden-final.txt && echo GOLDEN-STABLE-OK
git add docs/bench-baseline.md docs/follow-ups.md stg-world-design.md PROGRESS.md
git commit -m "docs: A2 bench 第三轮续表 + 前置小刀簿记收口(follow-ups 销 A1/A2、设计文档 ItemTypeCfg 七字段、PROGRESS)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: 全绿 + `GOLDEN-STABLE-OK`。

---

## Self-Review 记录(plan 作者自查)

1. **Spec 覆盖**:§2.1 表列+bump+判别测试 → Task 1/2;§2.2 bench → Task 4;§2.3 new_game → Task 3;§0 勘误验收①均匀漂移 → Task 1 Step 7,②不变性 → Task 2,不变流 → Task 3 Step 5 / Task 4 Step 4。无缺口。
2. **占位符**:Task 4 Step 2 的 `X.XX` 是"抄实测"的显式指令并注明不得残留,非 TBD。
3. **类型一致性**:`sprite: u16`/`TABLE_VERSION=2`/`new_game(u64, i32, &EclImage) -> Result<Box<World>, TaskStartError>` 全文一致;测试 helper 名 `root_and_async_image()`/`build_tables_v0()`/`step_with_director` 均对代码核实过(行号见各 Task)。
