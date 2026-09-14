# 玩法刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (inline；用户定：任务间审阅从简，不派逐任务复审子 agent). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 gameplay-design §1–§4 的三件时间工具落进引擎：停止（时停 + 触碰消弹，库存）/ 跳躍冷却 / 死亡即遡行 + 偏差值，Godot 里可玩。

**Architecture:** 全部在既有相位函数内改——A 组 `try_stop` 取代 bomb 与旧时停；相位 6/7/9 的「玩家技能冻结」早退改成只跑新碰撞行 8 的冻结分支；决死窗口耗尽由 `commit_death` 原地发遡行请求，timeline 认领路径不动，代价在 `rewind_landed` 从快照重算。step 顺序不变。

**Tech Stack:** Rust 1.94（stg-core / stg-harness / stg-godot gdext）、GDScript（Godot 4.6）。

**Spec:** `docs/superpowers/specs/2026-09-14-gameplay-tools-design.md`

## Global Constraints

- stg-core 断层线：不引入浮点 / 时钟 / 宿主 RNG / 无序容器（CLAUDE.md I1–I7）。
- 新 `PlayerState` 字段自动进校验和（derive），`copy_into` 走整块 `d.players = s.players`；尺寸哨兵 `world_size_sentinel_guards_copy_into_field_list` 可能因 padding **不响**（D20）——每次布局变化手工核实测值并补账目注释。
- 退役号一律**不复用**：输入位 7（`BTN_TIMESTOP`）、9（`BTN_REWIND`）；syscall 513；`LIFE_RESPAWNING` 值 3。
- 常量：`TIMESTOP_FRAMES = 180`（不变）、`JUMP_FRAMES = 30`（不变）、`REWIND_INVULN = 30`（不变）、`RESPAWN_INVULN = 120`（不变，改为续关专用）、新 `JUMP_COOLDOWN: u16 = 600`、新 `STOP_STOCK_MAX: u8 = 5`、新 `STOP_TOUCH_SCORE: u64 = 10`、`PIECES_PER_BOMB` 5 → 4、`Loadout::default().bombs` 3 → 2。
- `ENGINE_VER` 19 → 20（Task 6 一次 bump）；`TABLE_VERSION` 4 → 5；`LOG_FILE_VER` 1 → 2。
- 清理临时/变异代码**只用编辑撤回，禁止 `git checkout <file>` / `git stash`**（本仓吃过两次亏）。金向量对拍 base 已存 scratchpad `golden_base.txt`（md5 `978522fd…`）。
- commit 结尾：`Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`。
- 每个 Task 结束 `cargo test --workspace` 全绿再提交（Task 1/5 期间桥 crate 须同步删常量以保持 workspace 可编译；GDScript 侧到 Task 7/8 才改，中途不跑 Godot 冒烟）。

## File Map

| 文件 | 职责 / 本刀改动 |
|---|---|
| `crates/stg-core/src/input.rs` | 词表：删位 7、9；测试钉值 |
| `crates/stg-core/src/player.rs` | `PlayerState` 删 3 加 2 字段；`Loadout` 删 `time_stops`；新常量；删 `LIFE_RESPAWNING` |
| `crates/stg-core/src/world/player.rs` | 相位 3：`try_stop`、C 组冷却计时、`commit_death` 重写、`rewind_landed` 重写、`try_continue`；删 `try_bomb`/`try_time_stop`/`try_rewind`；测试大换血 |
| `crates/stg-core/src/spell.rs` | 资格轮询去 bomb；新 `void_spell_captures` |
| `crates/stg-core/src/tables.rs` + `src/tables/tables_v0.bin` | 删 `BombCfg` 族与编解码；`TABLE_VERSION` 5；重烘 |
| `crates/stg-core/src/world/integrate.rs` | 删 `attract_all_items` 及其测试 |
| `crates/stg-core/src/events.rs` | `ROW_STOP_TOUCH = 8`；`EVT_REWIND_REQUESTED` 文档改产出者 |
| `crates/stg-core/src/world/{collide,settle,cleanup}.rs` | 行 8 冻结分支三件 |
| `crates/stg-core/src/items.rs` | `PIECES_PER_BOMB = 4` |
| `crates/stg-core/src/ecl/syscall.rs` | 删 513；`SYS_ADD_BOMBS` 钳 5 |
| `crates/stg-ecl-compiler/src/lang/builtins.rs` | 删 `add_time_stops`；`add_bombs` doc |
| `crates/stg-core/src/step.rs` | `new_game_at` 钳库存、删 `time_stops`；冻结测试改口径；尺寸哨兵账；`engine_ver_anchored` |
| `crates/stg-core/src/timeline.rs` | 回放头删 1 字节 `LOG_FILE_VER=2`；遡行测试改为死亡触发 |
| `crates/stg-core/src/lib.rs` | `ENGINE_VER = 20` + 文档段 |
| `crates/stg-harness/src/replay.rs` | demo 回放闸改死亡触发 + 成功跳躍计数 |
| `crates/stg-godot/src/bridge.rs` + `smoke/smoke.gd` | 常量增删、`hud_player` 两键；桥冒烟改遡行/续关/默认库存 |
| `godot/scripts/{input,hud,play}.gd` | 键位、HUD 三件、観測窗口 180、提示文案、目验帧名 |
| 文档 | `docs/ecl-ops.md`、`docs/ecl-lang/{6-spell-and-stage,7-reference}.md`、`editors/vscode/stg-ecl/ecl-meta.json`（生成）、`docs/render-contract.md`、`docs/gameplay-design.md`、`docs/follow-ups.md`、`stg-world-design.md`、`CLAUDE.md`、`PROGRESS.md` |

---

### Task 1: 停止合并——`try_stop` 取代 bomb 与旧时停，删 bomb 全族

**Files:**
- Modify: `crates/stg-core/src/input.rs`（删 `BTN_TIMESTOP`；测试）
- Modify: `crates/stg-core/src/player.rs`（删 `bomb_phase`/`bomb_timer`/`time_stops`、`Loadout.time_stops`）
- Modify: `crates/stg-core/src/world/player.rs`（`try_stop`；删 `try_bomb`/`try_time_stop`、C 组 bomb 计时、`try_continue` 三行；测试）
- Modify: `crates/stg-core/src/spell.rs:91-94`（去 `bomb_phase`；加 `void_spell_captures`）
- Modify: `crates/stg-core/src/tables.rs`（删 `BombCfg`/`BombField`/`BombOrigin`、`CharacterCfg.bomb`、validate 段、`to_bytes`/`from_bytes` bomb 段、4 条 bomb 测试；`TABLE_VERSION` 5）
- Regenerate: `crates/stg-core/src/tables/tables_v0.bin`
- Modify: `crates/stg-core/src/world/integrate.rs`（删 `attract_all_items` + `attract_all_items_api_contract`）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（删 `SYS_ADD_TIME_STOPS` 常量/白名单/处理臂/名表行/测试）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（删 `add_time_stops` 条目 + 两份测试名单里的该名）
- Modify: `crates/stg-core/src/step.rs:127`（删 `p.time_stops = ...`）
- Modify: `crates/stg-core/src/timeline.rs:529-566,660-680`（回放头删 `time_stops` 字节，`LOG_FILE_VER = 2`）
- Modify: `crates/stg-godot/src/bridge.rs:118-119,275-276`（删 `BTN_TIMESTOP` 常量与 `time_stops` 注释）

**Interfaces:**
- Produces: `WorldBody::try_stop(&mut self, i: usize)`（私有，A 组）；`WorldBody::void_spell_captures(&mut self)`（`pub(crate)`，Task 5 复用）；输入位 `BTN_BOMB`（位 5）= 停止。

- [ ] **Step 1: 写停止的失败测试**（替换 `world/player.rs` 测试模块里 `// ── 时停自机入口` 与 `// ── bomb 自机入口` 两整段，即 `fn press` 起到模块末 `bomb_at_rank3_peak...` 止，全部删掉换成下面这段）

```rust
    // ── 停止自机入口（玩法刀 2026-09-14：时停 + bomb 合一，X = BTN_BOMB）──────────

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

    /// 门禁 + 效果：扣一发库存、写玩家技能倒计时、不碰 ECL 演出格。两件都断——
    /// 「扣费但没生效」「生效但没扣费」各能溜过只断其一的写法。
    #[test]
    fn stop_triggers_charges_one_and_freezes() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "应扣一发");
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
        assert_eq!(w.body.freeze_left[1], 0, "不得碰 ECL 演出那一格");
    }

    /// 停止期间再按（松手重按 = 真沿）= no-op 且不扣、不刷新倒计时。
    #[test]
    fn stop_reentry_is_free_noop() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        press(&mut w, 0);
        let left = w.body.freeze_left[0];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "停止中再按不得扣");
        assert!(w.body.freeze_left[0] < left, "也不得刷新倒计时");
    }

    #[test]
    fn stop_without_stock_does_nothing() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 0;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.freeze_left[0], 0);
    }

    /// A 组被 ECL 演出定住时发不出——A 组门禁自动给的。
    #[test]
    fn stop_is_unavailable_while_the_actor_is_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        w.body.freeze_left = [0, 10];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.freeze_left[0], 0, "被定住期间不得发动");
        assert_eq!(w.body.players[0].bombs, 2, "也不得扣");
    }

    /// 真实按键 ⇒ 世界恰好少走 `TIMESTOP_FRAMES` 帧（N±1 判别，观测面 = 有速度的敌弹）。
    /// 弹放在远离自机处（x=150），免得被 Task 3 的触碰消弹吃掉。
    #[test]
    fn real_button_press_skips_exactly_timestop_frames() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 150, 100);
        let bi = w.body.bullets.get(h).unwrap();
        w.body.bullets.vy[bi] = crate::math::Fx::from_int(1);
        let y0 = w.body.bullets.y[bi];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.bullets.y[bi], y0, "触发当帧起即已冻");
        for k in 1..crate::player::TIMESTOP_FRAMES {
            press(&mut w, 0);
            assert_eq!(w.body.bullets.y[bi], y0, "第 {k} 帧仍应冻结");
        }
        press(&mut w, 0);
        assert_ne!(w.body.bullets.y[bi], y0, "第 TIMESTOP_FRAMES+1 帧必须已解除");
        assert_eq!(w.body.freeze_left[0], 0);
    }

    /// 按住跨过整个冻结窗口（含解冻那帧）只扣一发——查电平的实现会在解冻帧再点一次。
    #[test]
    fn holding_through_expiry_consumes_exactly_one_charge() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        for _ in 0..=crate::player::TIMESTOP_FRAMES {
            press(&mut w, BTN_BOMB);
        }
        assert_eq!(w.body.players[0].bombs, 1, "全程按住只扣一发");
        assert_eq!(w.body.freeze_left[0], 0, "不得被电平误判重新点燃");
    }

    /// 与上条互补：松手、等窗口跑完再按 = 合法第二发。
    #[test]
    fn genuine_second_press_after_release_fires_again() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        for _ in 0..crate::player::TIMESTOP_FRAMES {
            press(&mut w, 0);
        }
        assert_eq!(w.body.freeze_left[0], 0);
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "合法第二发照常发动");
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
    }

    /// deathstop：决死窗口内按 X → 拨回 ALIVE、清窗口计时、扣一发、**不扣命**。
    #[test]
    fn deathstop_inside_the_window_revives_without_costing_a_life() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
        assert_eq!(w.body.players[0].lives, lives0, "不扣命");
        assert_eq!(w.body.players[0].state_timer, 0);
        assert_eq!(w.body.players[0].bombs, 0);
        assert_eq!(w.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
    }

    /// 窗口耗尽（命已扣）之后再按停止：命不会退回。与上一条成对才有判别力。
    #[test]
    fn stop_after_the_window_closed_cannot_undo_the_death() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0); // 窗口耗尽 → commit_death
        assert_eq!(w.body.players[0].lives, lives0 - 1, "已经扣命");
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "命不会退");
    }

    /// 停止 ⇒ active 符卡当场失格（资格轮询住 settle，冻结期间不跑，必须在触发点写）。
    #[test]
    fn stop_voids_the_spell_capture_at_trigger() {
        let mut w = crate::step::World::new(1);
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        // (slot, boss, spell_id, time_limit, bonus0, flags, hp_threshold)
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        assert_ne!(w.body.spells[0].capture_ok, 0, "前提：开卡时资格在");
        w.body.players[0].bombs = 1;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.spells[0].capture_ok, 0, "停止即失格");
    }
```

同时把测试模块头的 import 改为 `use crate::input::{BTN_BOMB, BTN_JUMP, BTN_REWIND, InputFrame};`（不变），并把 `bomb_wins_over_rewind_in_the_same_frame` 改名 `stop_wins_over_rewind_in_the_same_frame`，断言文案改「deathstop 救人 / 停止优先」（逻辑不变，Task 5 会整条删）。`continue_restores_defaults...` 里删 `assert_eq!(p.time_stops, ld.time_stops);`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib world::player::tests::stop_ 2>&1 | tail -20`
Expected: 编译失败或 FAIL（`try_stop` 不存在，`BTN_BOMB` 仍走 bomb 铺 field 路径，`stop_triggers_charges_one_and_freezes` 的 `freeze_left[0]` 断言红）。

- [ ] **Step 3: 实现核侧**

`input.rs`：删 `BTN_TIMESTOP = 7, Edge;` 连同上方两行文档；`BTN_BOMB` 文档改为：
```rust
    /// 停止（沿触发；消费者：`world/player.rs::try_stop`）。玩法刀 2026-09-14：时停 + 触碰消弹合一，
    /// 库存 = `PlayerState.bombs`。位 7（旧 `BTN_TIMESTOP`）退役不复用。
    BTN_BOMB = 5, Edge;
```
测试：`action_bit_values_frozen` 删 `BTN_TIMESTOP` 行，加注释 `// 位 7 退役（旧 BTN_TIMESTOP，玩法刀）：不复用`；`edge_mask_is_exactly_the_five_edge_actions` 改名 `edge_mask_is_exactly_the_edge_actions`，期望 `BTN_BOMB | BTN_JUMP | BTN_REWIND | BTN_CONTINUE`；`actions_table_matches_constants` 期望数组删 `BTN_TIMESTOP` 行、长度 10；`vocab_hash_pinned` 先保留旧值（Step 4 按实测改）。

`player.rs`：删字段 `bomb_phase`、`bomb_timer`、`time_stops`（及 `spawn` 里三行）；`Loadout` 删 `time_stops` 字段与 `Default` 里 `time_stops: 1`；`Loadout::default` 文档改「机体0/0火力/3残/3停止」（Task 2 改 2）；`TIMESTOP_FRAMES` 文档删「迁进 `CharacterCfg` 的路径与 `BombCfg` 完全同构」一句。

`spell.rs` 第 91–94 行改为：
```rust
            // 1. 资格轮询作废（先于一切）：中弹入决死窗 → capture_ok 清 0。停止不在这里——
            //    冻结期间 settle 不跑，由 `try_stop` 触发点调 `void_spell_captures`（玩法刀）。
            if self.players[0].life_state != LIFE_ALIVE {
                self.spells[slot].capture_ok = 0;
            }
```
并在同一 `impl WorldBody` 块末尾加：
```rust
    /// 全部 active 符卡槽当场失格（玩法刀）。两个调用点：`try_stop`（冻结期间轮询不跑）、
    /// `rewind_landed`（快照带回了被弹前的资格）。
    pub(crate) fn void_spell_captures(&mut self) {
        for s in self.spells.iter_mut() {
            if s.active != 0 {
                s.capture_ok = 0;
            }
        }
    }
```

`world/player.rs`：
- 删 C 组 `// bomb 计时归 C 组` 那整块 `if self.players[i].bomb_timer > 0 { ... }`；`LIFE_ALIVE` 臂注释 `// bomb 无敌（本切片恒 0）` → `// 遡行落地 / 续关无敌`。
- A 组 `self.try_time_stop(i); self.try_bomb(i, tables);` 两行换成 `self.try_stop(i);`。
- 删 `fn try_time_stop` 与 `fn try_bomb`（含文档），在原位置加：
```rust
    /// 停止触发（A 组，玩法刀 spec §2.2）：时停 + 触碰消弹合一，库存 = `bombs`。门禁四条：
    /// 上升沿（`pressed_edge`，按住跨窗口不连发）+ 库存 > 0 + 未在停止中（停止中再按 no-op
    /// 不扣）+ ALIVE 或决死窗口（deathstop）。deathstop 不退款：进窗口时命没扣，扣命只在
    /// `commit_death`。符卡资格在**触发点**作废——冻结期间 settle 不跑，轮询看不到。
    fn try_stop(&mut self, i: usize) {
        if !self.pressed_edge(i, crate::input::BTN_BOMB)
            || self.players[i].bombs == 0
            || self.freeze_left[0] != 0
            || !matches!(self.players[i].life_state, LIFE_ALIVE | LIFE_DEATHWINDOW)
        {
            return;
        }
        self.players[i].bombs -= 1;
        self.freeze_left[0] = crate::player::TIMESTOP_FRAMES;
        if self.players[i].life_state == LIFE_DEATHWINDOW {
            self.players[i].life_state = LIFE_ALIVE;
            self.players[i].state_timer = 0;
        }
        self.void_spell_captures();
    }
```
- `try_continue` 删 `p.time_stops = ld.time_stops;`、`p.bomb_phase = 0;`、`p.bomb_timer = 0;`；文档「残机 / bomb / 时停回」→「残机 / 停止库存回」。
- 文件头 `use` 去掉不再用到的项（编译器提示为准）。

`world/integrate.rs`：删 `pub(crate) fn attract_all_items`（含文档）与测试 `attract_all_items_api_contract`。

`tables.rs`：删 `BombCfg`/`BombField`/`BombOrigin` 三个类型、`CharacterCfg.bomb` 字段与其文档、`build_tables_v0` 里 `bomb: BombCfg { ... },` 整块、`validate` 里 `// bomb 描述层校验` 整块、`to_bytes` 里 `// bomb 段` 整块、`from_bytes` 里 `// bomb 段` 解析块与 `bomb: BombCfg {...}` 构造行、测试 `bomb_cfg_v0_is_two_fields`/`bomb_cfg_validate_rejects_bad_rows`/`bomb_cfg_survives_a_bytes_roundtrip`/`bomb_origin_rejects_unknown_discriminant`；`const TABLE_VERSION: u16 = 5;`。`TableLoadError::BadDiscriminant` 若编译器报无构造者警告，保留变体并在其文档加「当前无产出者（bomb_origin 随玩法刀退役），留给下一个枚举字段」。

`ecl/syscall.rs`：删 `SYS_ADD_TIME_STOPS` 常量（原处留注释 `// 513 退役（玩法刀 2026-09-14，原 add_time_stops）：号不复用。`）、白名单 `| SYS_ADD_TIME_STOPS`、处理臂、名表测试行 `(SYS_ADD_TIME_STOPS, "add_time_stops", 5),`、测试 `sys_add_time_stops_saturates_like_its_siblings` 与其节标题注释。

`builtins.rs`：删 `name: "add_time_stops"` 整个 `Builtin {..}`；两份测试名单里删 `"add_time_stops",`；`// ── 自机能力刀：时停（syscall 513/560）` 注释改 `（syscall 560）`。

`step.rs::new_game_at`：删 `p.time_stops = loadout.time_stops;`。

`timeline.rs`：格式注释 `bombs u8, time_stops u8}` → `bombs u8}`、`InputLog 字节格式 v1` → `v2`；`LOG_FILE_VER: u8 = 2;`；`to_bytes` 删 `out.push(loadout.time_stops);`；`from_bytes` 删 `let time_stops = ...` 与 `Loadout { ..., time_stops }` 里那项。

`stg-godot/src/bridge.rs`：删 `#[constant] const BTN_TIMESTOP ...` 两行；`new_game_at` 的 Loadout 字面量删「时停刀（裁定 R-2）」两行注释（保留 `..Default::default()`）。

- [ ] **Step 4: 重烘表 + 跑全量，修实测钉值**

```bash
cargo run -q -p stg-harness -- bake-tables
cargo run -q -p stg-harness -- verify-tables
cargo test --workspace 2>&1 | grep -E '^test result|FAILED|panicked' 
```
Expected: 除下列「实测钉值」外全绿：
- `input::tests::vocab_hash_pinned` —— 把断言值改为失败信息里的实测值，注释补一行「玩法刀删 BTN_TIMESTOP=7 → 0x...」。
- `step::tests::world_size_sentinel_guards_copy_into_field_list` —— 若红：`PlayerState` 删了 u8+u16+u8；按失败信息的实测 `(WorldBody, World)` 更新数字，并在账目注释末尾追加一段「2026-09-14（玩法刀 Task 1）：`PlayerState` 删 `bomb_phase: u8`/`bomb_timer: u16`/`time_stops: u8` ⇒ 实测 X→Y；① copy_into 整块 Copy 无需同步 ② checksum/SaveBytes derive 自动 ③ D10 不适用 ④ wire format 变，ENGINE_VER 在 Task 6 统一 bump」。若绿：同样追加一段写明「size_of 实测未变（padding 吸收），D20 盲区」。
- 其余任何红：按 spec 语义修测试或实现，不改断言意图。

- [ ] **Step 5: clippy + 提交**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
cargo fmt --all
git add -A crates && git commit -m "feat(core): 玩法刀 T1——停止合并（try_stop 取代 bomb/时停，BTN_TIMESTOP 位 7 与 syscall 513 退役，删 BombCfg 族，表 v5 重烘，回放头 v2）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: 停止库存数字——碎片 4、上限 5、默认 2

**Files:**
- Modify: `crates/stg-core/src/items.rs:34`
- Modify: `crates/stg-core/src/player.rs`（`STOP_STOCK_MAX`、`Loadout::default().bombs = 2`）
- Modify: `crates/stg-core/src/world/settle.rs:297-305`（进位钳）+ 测试
- Modify: `crates/stg-core/src/ecl/syscall.rs:863-868`（钳 5）+ 测试 `add_bombs_clamps_both_ends`、`add_counters_survive_extreme_deltas`
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（`add_bombs` doc）
- Modify: `crates/stg-core/src/step.rs`（`new_game_at` 钳；测试 `new_game_at_applies_loadout_with_clamp`）

**Interfaces:**
- Produces: `pub const STOP_STOCK_MAX: u8 = 5;`（`crate::player`，Task 7 桥导出）。

- [ ] **Step 1: 写失败测试**

`settle.rs` 测试模块加：
```rust
    /// 停止碎片（玩法刀）：4 枚进 1 发；库存已满时碎片照清、不加（判别腿：满库存 5 与 4 各一次）。
    #[test]
    fn stop_piece_carry_respects_stock_max() {
        use crate::items::{ITEM_BOMB_PIECE, PIECES_PER_BOMB};
        use crate::player::STOP_STOCK_MAX;
        assert_eq!(PIECES_PER_BOMB, 4, "gameplay-design §1：4 碎片 = 1 发");
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = STOP_STOCK_MAX - 1;
        w.body.players[0].bomb_pieces = PIECES_PER_BOMB - 1;
        w.body.credit_item(0, ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "未满：进位加一");
        assert_eq!(w.body.players[0].bomb_pieces, 0);
        w.body.players[0].bomb_pieces = PIECES_PER_BOMB - 1;
        w.body.credit_item(0, ITEM_BOMB_PIECE, &crate::tables::TABLES_V0);
        assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "已满：不加");
        assert_eq!(w.body.players[0].bomb_pieces, 0, "已满：碎片照清");
    }
```
`credit_item_saturates_lives_and_bombs_at_u8_max` 的 bombs 半段断言文案改「库存超上限的直写值不被进位推高」（值仍 `u8::MAX`，逻辑成立）。

`syscall.rs` 的 `add_bombs_clamps_both_ends` 改为：
```rust
    /// `add_bombs(d)`：钳 `[0, STOP_STOCK_MAX=5]`（玩法刀；原 `[0,255]`）。判别腿：写错字段会让 lives 动。
    #[test]
    fn add_bombs_clamps_both_ends() {
        use crate::player::STOP_STOCK_MAX;
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.players[0].bombs = 3;
        let lives0 = w.body.players[0].lives;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[1]).is_ok());
        assert_eq!(w.body.players[0].bombs, 4, "正增");
        assert_eq!(w.body.players[0].lives, lives0, "判别腿：不得误写 lives");
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[-1]).is_ok());
        assert_eq!(w.body.players[0].bombs, 3, "负减");
        w.body.players[0].bombs = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[-1]).is_ok());
        assert_eq!(w.body.players[0].bombs, 0, "下钳 0");
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[9]).is_ok());
        assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "上钳 5");
    }
```
`add_counters_survive_extreme_deltas` 的循环里 `i32::MAX` 期望改为按字段分：`let cap = if no == SYS_ADD_LIVES { u8::MAX } else { crate::player::STOP_STOCK_MAX };` 断言 `v == cap`，文案 `"syscall {no}：i32::MAX 应钳到上限"`。

`step.rs` 的 `new_game_at_applies_loadout_with_clamp`：`bombs: 1` 改 `bombs: 9`，断言改 `assert_eq!(p.bombs, crate::player::STOP_STOCK_MAX, "bombs 钳到停止库存上限");`，文档首行改「power 越 `POWER_MAX` 钳、bombs 越 `STOP_STOCK_MAX` 钳、lives 全域直收」。

`player.rs` 测试模块加：
```rust
    #[test]
    fn default_loadout_starts_with_two_stops() {
        assert_eq!(Loadout::default().bombs, 2, "gameplay-design §1：初始 2");
    }
```

- [ ] **Step 2: 跑确认失败**

Run: `cargo test -p stg-core --lib 2>&1 | grep -E 'FAILED|panicked|error\[' | head`
Expected: `STOP_STOCK_MAX` 未定义编译错。

- [ ] **Step 3: 实现**

`items.rs`：`pub const PIECES_PER_BOMB: u8 = 4;`（文档补「停止碎片，玩法刀 5→4」）。
`player.rs`（`TIMESTOP_FRAMES` 下方）：
```rust
/// 停止库存上限（gameplay-design §1）。三个入口钳它：碎片进位、`SYS_ADD_BOMBS`、`new_game_at`。
pub const STOP_STOCK_MAX: u8 = 5;
```
`Loadout::default` 里 `bombs: 2`，文档「3残/2停止」。
`settle.rs` `ITEM_BOMB_PIECE` 臂：
```rust
                if pl.bomb_pieces >= PIECES_PER_BOMB {
                    pl.bomb_pieces = 0;
                    if pl.bombs < crate::player::STOP_STOCK_MAX {
                        pl.bombs += 1;
                    }
                }
```
`syscall.rs` `SYS_ADD_BOMBS` 臂 `.clamp(0, crate::player::STOP_STOCK_MAX as i32)`；号表注释改「停止库存增量（B20；玩法刀起钳 `[0, STOP_STOCK_MAX]`）」。
`builtins.rs` `add_bombs` doc：`"增减停止库存:delta 允许负,双边钳 [0,STOP_STOCK_MAX=5] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_"`。
`step.rs::new_game_at`：`p.bombs = loadout.bombs.min(crate::player::STOP_STOCK_MAX);`。

- [ ] **Step 4: 全量**

Run: `cargo test --workspace 2>&1 | grep -E '^test result|FAILED|panicked'`
Expected: 全绿（`crates/stg-godot` 的 `boot` 测试不查 bombs）。若 `ecl-lang` 手册 ```ecl 围栏编译测试红（doc 字符串不影响编译，理论不会），按报错修。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && git add -A crates && git commit -m "feat(core): 玩法刀 T2——停止库存：碎片 4 进 1、上限 5（进位/add_bombs/new_game_at 三处钳）、默认 2

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: 触碰消弹——碰撞矩阵行 8 的冻结分支

**Files:**
- Modify: `crates/stg-core/src/events.rs`（`ROW_STOP_TOUCH`）
- Modify: `crates/stg-core/src/player.rs`（`STOP_TOUCH_SCORE`）
- Modify: `crates/stg-core/src/world/collide.rs:23-25`（冻结分支 + `collide_stop_touch`）
- Modify: `crates/stg-core/src/world/settle.rs:127-129`（冻结分支 + `settle_stop_touch`）
- Modify: `crates/stg-core/src/world/cleanup.rs:14-33`（冻结分支 + `cleanup_stop_touched` + 模块注释推论改口径）+ 测试 `vanished_is_empty_while_scene_frozen`
- Modify: `crates/stg-core/src/step.rs` 测试：`player_skill_freezes_scene_but_not_the_actor`、`player_skill_freeze_makes_the_player_untouchable`；新测试
- Test: `crates/stg-core/src/world/player.rs`（真按键端到端一条）

**Interfaces:**
- Consumes: `try_stop`（Task 1）。
- Produces: `pub(crate) const ROW_STOP_TOUCH: u8 = 8;`、`pub const STOP_TOUCH_SCORE: u64 = 10;`。

- [ ] **Step 1: 写失败测试**

`step.rs` 测试模块（`player_skill_freeze_makes_the_player_untouchable` 之后）加：
```rust
    /// 触碰几何判别（玩法刀 spec §2.4）：弹心距 == `br + hit_radius` 恰好碰到 → 消 +10；
    /// 多 1 raw → 不碰。`hit_radius` 2.5px + 哑弹半径 2px = 4.5px = 294_912 raw。
    /// 圆心重合式摆法对半径映射是瞎的（M0-7 教训），故两颗弹一左一右卡在边界两侧。
    #[test]
    fn stop_touch_clears_exactly_at_contact_distance_and_scores() {
        let mut w = World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        let sum = (w.body.players[0].hit_radius + Fx::from_int(2)).raw();
        let a = crate::world::test_support::bullet_at(&mut w, 0, 384);
        let b = crate::world::test_support::bullet_at(&mut w, 0, 384);
        let (ai, bi) = (w.body.bullets.get(a).unwrap(), w.body.bullets.get(b).unwrap());
        w.body.bullets.x[ai] = Fx::from_raw(sum);
        w.body.bullets.x[bi] = Fx::from_raw(-(sum + 1));
        let score0 = w.body.players[0].score;
        w.body.freeze_left = [10, 0];
        step_empty(&mut w);
        assert!(w.body.bullets.get(a).is_none(), "恰好相切：被消且当帧回收");
        assert!(w.body.bullets.get(b).is_some(), "差 1 raw：不碰");
        assert_eq!(
            w.body.players[0].score,
            score0 + crate::player::STOP_TOUCH_SCORE,
            "每弹 +10"
        );
        let v = w.body.vanished();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].reason, crate::events::VANISH_CLEARED);
        assert_eq!(v[0].x, Fx::from_raw(sum), "vanished 记被消那颗的位置");
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
    }

    /// delay 弹（未出生）不参与触碰——与行 1 同口径。
    #[test]
    fn stop_touch_skips_delay_bullets() {
        let mut w = World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        let h = crate::world::test_support::bullet_at(&mut w, 0, 384);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.delay[i] = 5;
        w.body.freeze_left = [10, 0];
        step_empty(&mut w);
        assert!(w.body.bullets.get(h).is_some());
    }
```
改 `player_skill_freeze_makes_the_player_untouchable`：末尾追加
```rust
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "玩法刀：压在身上的冻弹被触碰消掉"
        );
```
改 `player_skill_freezes_scene_but_not_the_actor` 最后一条：
```rust
        assert!(
            !w.body.bullets.is_alive(cleared_i),
            "相位 9 冻结分支（玩法刀）：已标记清除的弹冻结期间即回收"
        );
```
（`busy_world` 里若有压在自机 x=0 附近的弹，本条的 `bullet_y` 等断言会暴露——那就是行 8 真在跑，按实际把观测弹换成远离自机的那颗，不改断言意图。）

`cleanup.rs` 的 `vanished_is_empty_while_scene_frozen` 替换为：
```rust
    /// 冻 C（停止）时 cleanup 只收已清除弹：寿尽弹留池不记；已清除弹回收并记 `VANISH_CLEARED`。
    #[test]
    fn frozen_cleanup_only_recycles_cleared_bullets() {
        use crate::bullets::BULLET_CLEARED;
        use crate::events::VANISH_CLEARED;
        #[cfg(debug_assertions)]
        use crate::world::PH_CLEANUP;
        let mut w = crate::step::World::new(1);
        let dead = crate::world::test_support::bullet_at(&mut w, 0, 100);
        let di = w.body.bullets.get(dead).unwrap();
        w.body.bullets.life[di] = 0;
        let cleared = crate::world::test_support::bullet_at(&mut w, 50, 100);
        let ci = w.body.bullets.get(cleared).unwrap();
        w.body.bullets.flags[ci] |= BULLET_CLEARED;
        w.body.freeze_left[0] = 5;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_CLEANUP;
        }
        w.body.cleanup();
        assert!(w.body.bullets.get(dead).is_some(), "寿尽弹冻结中不回收");
        assert!(w.body.bullets.get(cleared).is_none(), "已清除弹冻结中回收");
        assert_eq!(w.body.vanished().len(), 1);
        assert_eq!(w.body.vanished()[0].reason, VANISH_CLEARED);
    }
```
`world/player.rs` 停止测试段末尾加端到端：
```rust
    /// 真按键端到端：身上压一颗弹，按 X → 同帧冻结 + 触碰消掉 + 仍 ALIVE。
    #[test]
    fn stop_by_button_clears_the_bullet_under_the_player() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].bombs = 1;
        bullet_at(&mut w, 0, 384);
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
    }
```

- [ ] **Step 2: 跑确认失败**

Run: `cargo test -p stg-core --lib stop_touch 2>&1 | tail -15`
Expected: `STOP_TOUCH_SCORE` 未定义 → 编译错。

- [ ] **Step 3: 实现**

`events.rs` 行号块末加：
```rust
/// 停止冻结中：自机判定圆 × 冻住的敌弹 → 触碰消弹（玩法刀 2026-09-14）。只在 `scene_frozen()` 时收集。
pub(crate) const ROW_STOP_TOUCH: u8 = 8;
```
`player.rs`（`STOP_STOCK_MAX` 下）：
```rust
/// 停止中触碰消弹每颗的得分（gameplay-design §5）。
pub const STOP_TOUCH_SCORE: u64 = 10;
```
`collide.rs`：模块文档行表追加「行8 停止冻结中自机判定圆×冻弹（触碰消弹）」；`use` 加 `ROW_STOP_TOUCH`；门禁改
```rust
        if self.scene_frozen() {
            self.collide_stop_touch(); // 行 8：停止冻结中只收触碰消弹（玩法刀）
            return;
        }
```
并加：
```rust
    /// 行 8：停止冻结中，自机判定圆 × 冻住的敌弹。只 ALIVE 参与、**不看 `invuln`**（停止本身
    /// 就是无敌窗，落地无敌帧不该让触碰失效）；delay 弹跳过（与行 1 同口径）。半径和比较同行 1。
    fn collide_stop_touch(&mut self) {
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE {
                continue;
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let nw = self.bullets.alive.len();
            for w in 0..nw {
                let mut bits = self.bullets.alive[w];
                while bits != 0 {
                    let b = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if self.bullets.delay[b] > 0 {
                        continue;
                    }
                    let d2 = len_sq(self.bullets.x[b] - px, self.bullets.y[b] - py);
                    let sum = (self.bullets.radius[b] + hit_r).raw() as i64;
                    if d2 <= sum * sum {
                        self.push_hit(ROW_STOP_TOUCH, b as u16, p as u16);
                    }
                }
            }
        }
    }
```
`settle.rs`：门禁改
```rust
        if self.scene_frozen() {
            self.settle_stop_touch(); // 行 8 结算（玩法刀）；符卡趟照旧不跑 ⇒ 停止不烧符卡时间
            return;
        }
```
并加（`credit_item` 之前）：
```rust
    /// 行 8 结算（停止冻结中）：未清除的弹置 `BULLET_CLEARED` + 该自机 `STOP_TOUCH_SCORE`。
    /// 不转星星、不发事件；同一颗弹多个自机碰到按 hits 序首个入账（I4）。回收在相位 9 冻结分支。
    fn settle_stop_touch(&mut self) {
        for k in 0..self.hits_len as usize {
            let h = self.hits[k];
            if h.row != crate::events::ROW_STOP_TOUCH {
                continue;
            }
            let b = h.active as usize;
            if self.bullets.flags[b] & crate::bullets::BULLET_CLEARED != 0 {
                continue;
            }
            self.bullets.flags[b] |= crate::bullets::BULLET_CLEARED;
            let p = h.passive as usize;
            self.players[p].score = self.players[p]
                .score
                .saturating_add(crate::player::STOP_TOUCH_SCORE);
        }
    }
```
`cleanup.rs`：模块/函数内「C 组：冻 C 时没有新的越界/消弹/死亡标记产生…残留」那段第一句改为「冻 C（停止）时只回收已清除弹（行 8 触碰消掉的、以及冻结前一帧被清的——后者从"残留到解冻"改为冻结首帧即收，玩法刀）；越界/寿尽/dying 敌照旧留到解冻」，`signals[]` 那段不动；门禁改
```rust
        if self.scene_frozen() {
            self.cleanup_stop_touched();
            return;
        }
```
并加：
```rust
    /// 冻结分支（玩法刀）：只收带 `BULLET_CLEARED` 的弹，写 `VANISH_CLEARED`（冻结中弹不动，必在场内），
    /// 释放 xform 段。寿命/越界/自机弹/敌/道具/作用区一律留到解冻。
    fn cleanup_stop_touched(&mut self) {
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.bullets.flags[i] & crate::bullets::BULLET_CLEARED == 0 {
                    continue;
                }
                self.push_vanished(
                    self.bullets.x[i],
                    self.bullets.y[i],
                    self.bullets.sprite[i],
                    crate::events::VANISH_CLEARED,
                );
                if self.bullets.transform_head[i] != crate::xform::XFORM_NONE {
                    self.xforms.free(self.bullets.transform_head[i]);
                }
                self.bullets.free_index(i);
            }
        }
    }
```

- [ ] **Step 4: 全量**

Run: `cargo test --workspace 2>&1 | grep -E '^test result|FAILED|panicked'`
Expected: 全绿。`full_freeze_changes_nothing_but_the_always_running_fields` 保持绿 = 「无触碰时整块逐位不变」的押运仍成立；若它红，说明 `busy_world` 有弹压在自机上——把该测试 `w.body.players[0].x` 挪到场外角落（如 `Fx::from_int(-190)`）再比，注释写明理由。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
git add -A crates && git commit -m "feat(core): 玩法刀 T3——停止触碰消弹：碰撞矩阵行 8（相位 6/7/9 冻结分支，判定圆×冻弹，+10 分，当帧回收记 VANISH_CLEARED）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: 跳躍冷却 `jump_cd`

**Files:**
- Modify: `crates/stg-core/src/player.rs`（字段 + `JUMP_COOLDOWN`）
- Modify: `crates/stg-core/src/world/player.rs`（C 组计时、JUMPING 落地写、`try_jump` 门禁）+ 测试
- Modify: `crates/stg-core/src/timeline.rs` 测试（`preview_jumps_even_if_jump_key_is_held_in_the_real_world` 清冷却；新影子口径测试）
- Modify: `crates/stg-core/src/step.rs`（尺寸哨兵账，如响）

**Interfaces:**
- Produces: `PlayerState.jump_cd: u16`、`pub const JUMP_COOLDOWN: u16 = 600;`（Task 7 桥导出）。

- [ ] **Step 1: 写失败测试**（`world/player.rs` 跳躍段）

`jump_enters_jumping_and_returns_alive_after_exactly_n_frames` 末尾（回 ALIVE 断言之后）加：
```rust
        assert_eq!(
            w.body.players[0].jump_cd,
            crate::player::JUMP_COOLDOWN,
            "落地那帧写满冷却"
        );
```
新增：
```rust
    /// 冷却判别（落地后）：再走 598 帧（cd=2）按 JUMP 仍 ALIVE；再走 599 帧（cd=1）按 JUMP——
    /// 本帧 C 组先减到 0、A 组门禁放行 ⇒ 起跳。两腿夹住「恰好 600 帧」。
    #[test]
    fn jump_cooldown_blocks_until_exactly_expired() {
        use crate::player::{JUMP_COOLDOWN, LIFE_ALIVE};
        let land = || {
            let mut w = crate::step::World::new(1);
            step_t(&mut w, &keys(BTN_JUMP));
            for _ in 0..JUMP_FRAMES {
                step_t(&mut w, &InputFrame::empty(0));
            }
            assert_eq!(w.body.players[0].jump_cd, JUMP_COOLDOWN);
            w
        };
        let mut w = land();
        for _ in 0..(JUMP_COOLDOWN - 2) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE, "cd 未尽不得跳");
        let mut w = land();
        for _ in 0..(JUMP_COOLDOWN - 1) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        step_t(&mut w, &keys(BTN_JUMP));
        assert_eq!(w.body.players[0].life_state, LIFE_JUMPING, "cd 恰尽即可跳");
    }

    /// 冷却归 C 组：停止冻结期间不走。
    #[test]
    fn jump_cooldown_does_not_tick_while_scene_frozen() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].jump_cd = 100;
        w.body.freeze_left = [11, 0];
        for _ in 0..10 {
            step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].jump_cd, 100);
    }

    #[test]
    fn jump_cd_enters_the_checksum() {
        let mut w = crate::step::World::new(1);
        let c0 = w.checksum();
        w.body.players[0].jump_cd = 1;
        assert_ne!(w.checksum(), c0);
    }
```
`timeline.rs`：`preview_jumps_even_if_jump_key_is_held_in_the_real_world` 在 `assert_ne!(... prev_input & BTN_JUMP, 0);` 之后加
```rust
        // 冷却是另一条门禁（玩法刀），本条只押「旧电平被清」——清掉冷却再预览。
        t.world.body.players[0].jump_cd = 0;
```
并新增：
```rust
    /// 冷却中観測（玩法刀 spec §3）：影子喂的 JUMP 被门禁拒，影子自机仍在场。
    #[test]
    fn preview_during_cooldown_keeps_the_shadow_player_present() {
        let mut t = bare(15);
        t.world.body.players[0].jump_cd = 100;
        t.preview_begin();
        t.preview_step();
        assert_eq!(t.shadow().body.players[0].life_state, LIFE_ALIVE);
    }
```

- [ ] **Step 2: 跑确认失败**

Run: `cargo test -p stg-core --lib jump_cooldown 2>&1 | tail -10`
Expected: 编译错 `no field jump_cd`。

- [ ] **Step 3: 实现**

`player.rs`：`JUMP_FRAMES` 下加
```rust
/// 跳躍冷却（帧，gameplay-design §2）：落地那帧写满，C 组逐帧减，`== 0` 才能再跳。
pub const JUMP_COOLDOWN: u16 = 600;
```
`PlayerState` 在 `invuln: u16` 之后加
```rust
    /// 跳躍冷却剩余帧（玩法刀）。落地写 `JUMP_COOLDOWN`，C 组计时（停止冻结期间不走）。
    pub jump_cd: u16,
```
`spawn` 加 `jump_cd: 0,`。
`world/player.rs` C 组 `if !scene {` 内、`match` 之前加：
```rust
                // 跳躍冷却（玩法刀）：先减后判——落地帧的写满在下面 JUMPING 臂，本帧不被自己减掉。
                if self.players[i].jump_cd > 0 {
                    self.players[i].jump_cd -= 1;
                }
```
`LIFE_JUMPING` 臂：
```rust
                        if self.players[i].state_timer == 0 {
                            self.players[i].life_state = LIFE_ALIVE;
                            self.players[i].jump_cd = crate::player::JUMP_COOLDOWN;
                        }
```
`try_jump` 门禁加 `|| self.players[i].jump_cd != 0`，文档「门禁三条」→「门禁四条：…+ 冷却已尽（玩法刀）」。

- [ ] **Step 4: 全量 + 哨兵**

Run: `cargo test --workspace 2>&1 | grep -E '^test result|FAILED|panicked'`
Expected: 全绿，除可能的 `world_size_sentinel_*`（按 Task 1 Step 4 同法更新实测值并追加账目「玩法刀 Task 4：`jump_cd: u16`」）。`timeline::tests::replay_reproduces_live_checksums_including_jumps_and_rewinds` 的 `jumps >= 2` 计的是尝试次数，此处仍绿；Task 5 会改成成功次数。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && git add -A crates && git commit -m "feat(core): 玩法刀 T4——跳躍冷却 jump_cd（落地写 600，C 组计时，冻结不走，影子冷却中不跳）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: 死亡即遡行 + 偏差值；退役遡行键与 RESPAWNING

**Files:**
- Modify: `crates/stg-core/src/player.rs`（删 `LIFE_RESPAWNING`、加 `deaths`、`RESPAWN_INVULN` 文档）
- Modify: `crates/stg-core/src/input.rs`（删 `BTN_REWIND`；测试）
- Modify: `crates/stg-core/src/events.rs:62-68`（`EVT_REWIND_REQUESTED` 文档）
- Modify: `crates/stg-core/src/world/player.rs`（C 组删 RESPAWNING 臂、`commit_death`、删 `try_rewind`、`rewind_landed`、`try_continue`；测试）
- Modify: `crates/stg-core/src/timeline.rs`（模块文档；遡行测试族改死亡触发；新测试）
- Modify: `crates/stg-harness/src/replay.rs`（demo 回放闸）
- Modify: `crates/stg-godot/src/bridge.rs:124-125`（删 `BTN_REWIND` 常量）
- Modify: `crates/stg-core/src/step.rs`（尺寸哨兵账，如响）

**Interfaces:**
- Consumes: `void_spell_captures`（Task 1）、`JUMP_COOLDOWN`（Task 4）。
- Produces: `PlayerState.deaths: u8`；`commit_death` 发 `EVT_REWIND_REQUESTED{a_index, x, y, data=[hit_frame,0]}`；`rewind_landed(i)` 语义：`deaths+1`、`lives=max(lives−1,1)`、`invuln=max(..,30)`、失格。

- [ ] **Step 1: 写失败测试**

`world/player.rs`：
- 删 `rewind_request_only_in_deathwindow_and_carries_hit_frame`、`stop_wins_over_rewind_in_the_same_frame`；测试模块 import 改 `use crate::input::{BTN_BOMB, BTN_JUMP, InputFrame};`。
- `jump_gate_rejects_non_alive_frozen_and_rejump`：删 `// RESPAWNING` 那一小段（3 行 + assert），`use` 去掉 `LIFE_RESPAWNING`，文档「DEATHWINDOW / RESPAWNING /」→「DEATHWINDOW /」。
- `rewind_landed_writes_invuln_and_rejects_bad_index` 替换为：
```rust
    /// 落地写 API（玩法刀 spec §4.3）：偏差值 +1、残机 −1 且下限 1、无敌取 max、active 卡失格；
    /// 越界自机号 → no-op + 违约计数（P4-b）。
    #[test]
    fn rewind_landed_pays_the_death_and_floors_lives_at_one() {
        let mut w = crate::step::World::new(1);
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        w.body.players[0].lives = 3;
        w.body.rewind_landed(0);
        let p = w.body.players[0];
        assert_eq!((p.lives, p.deaths, p.invuln), (2, 1, REWIND_INVULN));
        assert_eq!(w.body.spells[0].capture_ok, 0, "快照带回的资格被作废");
        w.body.players[0].lives = 1;
        w.body.rewind_landed(0);
        assert_eq!(w.body.players[0].lives, 1, "下限 1：致死与否只在死的那一刻判");
        assert_eq!(w.body.players[0].deaths, 2);
        let cv0 = w.body.diag.contract_viol;
        w.body.rewind_landed(crate::MAX_PLAYERS);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    }
```
- `deathwindow_expires_to_respawn_after_window` 替换为：
```rust
    /// 决死窗口耗尽（玩法刀 spec §4.2）：原地回 ALIVE、30 帧无敌、残机 −1、偏差值 +1，
    /// 同帧发 `EVT_PLAYER_DIED` 与 `EVT_REWIND_REQUESTED{data[0]=hit_frame}`；N−1 帧仍在窗口。
    #[test]
    fn deathwindow_expiry_continues_in_place_and_requests_rewind() {
        use crate::events::{EVT_PLAYER_DIED, EVT_REWIND_REQUESTED};
        use crate::math::Fx;
        use crate::player::{DEATHBOMB_WINDOW, LIFE_ALIVE, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].life_state = LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = DEATHBOMB_WINDOW;
        w.body.players[0].hit_frame = 7;
        w.body.players[0].x = Fx::from_int(50);
        w.body.players[0].y = Fx::from_int(300);
        let lives0 = w.body.players[0].lives;
        for _ in 0..(DEATHBOMB_WINDOW - 1) {
            step_t(&mut w, &InputFrame::empty(0));
        }
        assert_eq!(w.body.players[0].life_state, LIFE_DEATHWINDOW, "N−1 帧仍在窗口");
        step_t(&mut w, &InputFrame::empty(0));
        let p = w.body.players[0];
        assert_eq!(p.life_state, LIFE_ALIVE);
        assert_eq!((p.x, p.y), (Fx::from_int(50), Fx::from_int(300)), "原地，不回场底");
        assert_eq!(p.lives, lives0 - 1);
        assert_eq!(p.deaths, 1);
        assert_eq!(p.invuln, REWIND_INVULN);
        let evs = w.frame_events();
        assert!(evs.iter().any(|e| e.kind == EVT_PLAYER_DIED));
        let req: Vec<_> = evs.iter().filter(|e| e.kind == EVT_REWIND_REQUESTED).collect();
        assert_eq!(req.len(), 1);
        assert_eq!((req[0].a_index, req[0].data[0]), (0, 7), "载荷 = hit_frame");
    }
```
- 新增：
```rust
    #[test]
    fn deaths_enters_the_checksum() {
        let mut w = crate::step::World::new(1);
        let c0 = w.checksum();
        w.body.players[0].deaths = 1;
        assert_ne!(w.checksum(), c0);
    }
```
- `last_life_death_enters_gameover_and_freezes_player`：`GAMEOVER` 断言后加
```rust
        assert_eq!(w.body.players[0].deaths, 1, "最后一条命也计偏差值");
```
并在窗口循环里收集事件：把循环改为
```rust
        let mut requested = false;
        for f in 0..DEATHBOMB_WINDOW as u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            requested |= w
                .frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_REWIND_REQUESTED);
        }
        assert!(!requested, "残机耗尽不遡行");
```
文档里「上面那个测试走的是 3→2 的 RESPAWNING 臂」→「上面那个测试走的是 3→2 的原地继续臂」，「GAMEOVER（不是 RESPAWNING）」→「GAMEOVER（不遡行）」。
- `continue_restores_defaults_from_gameover_and_is_noop_otherwise`：`use` 去 `LIFE_RESPAWNING` 加 `LIFE_ALIVE` 与 `crate::math::Fx`；GAMEOVER 段在 `step_t` 前加 `w.body.players[0].x = Fx::from_int(-40); w.body.players[0].deaths = 3;`，断言改
```rust
        assert_eq!(p.life_state, LIFE_ALIVE, "续关原地复活（玩法刀：RESPAWNING 退役）");
        assert_eq!(p.x, Fx::from_int(-40), "位置不动");
        assert_eq!(p.deaths, 3, "偏差值不因续关洗白");
```
（删 `assert_eq!(p.life_state, LIFE_RESPAWNING);`，保留 `invuln == RESPAWN_INVULN` 等其余断言。）

`timeline.rs` 测试模块：`use crate::input::BTN_REWIND;` 删；加辅助
```rust
    /// 中弹后空跑到决死窗口耗尽 → 返回遡行 cut（玩法刀：死亡即遡行，无遡行键）。
    fn die_and_rewind(t: &mut Timeline) -> (u32, Cut) {
        for _ in 0..=DEATHBOMB_WINDOW {
            let at = t.frame() + 1;
            if let Some(c) = t.advance(&InputFrame::empty(0)).rewound {
                return (at, c);
            }
        }
        panic!("决死窗口耗尽必须遡行");
    }
```
- `rewind_restores_ring_frame_and_lands`：从 `for _ in 0..2 { advance }` 起到 `let cut = adv.rewound.expect(...)` 止替换为 `let (at, cut) = die_and_rewind(&mut t);`；在 `plant_hit` 之前记 `let snap_lives = t.ring_get(expect_to).unwrap().body.players[0].lives;`；`Cut{at,..}` 断言保留；落地断言后加 `assert_eq!(t.world().body.players[0].lives, snap_lives - 1); assert_eq!(t.world().body.players[0].deaths, 1);`；probe 还原改为
```rust
        probe.body.players[0].invuln = 0;
        probe.body.players[0].lives += 1;
        probe.body.players[0].deaths -= 1;
```
`ring_get_discarded(at - 1)` 保留。
- `early_rewind_clamps_to_oldest_frame`：`t.advance(&InputFrame::empty(0)); let adv = t.advance(&keys(BTN_REWIND));` 换成 `t.advance(&InputFrame::empty(0)); let (_, cut) = die_and_rewind(&mut t);`，断言 `assert_eq!(cut.to, 0);`；probe 同上三行还原。
- `seal_history_bounds_rewind_to_the_seal_frame`：同法换 `die_and_rewind`，断言 `cut.to == 50`。
- `no_request_means_no_rewind` 替换为：
```rust
    /// 最后一条命：窗口耗尽 → GAMEOVER，timeline 不遡行（玩法刀 spec §4.2 ②）。
    #[test]
    fn last_life_death_is_gameover_without_rewind() {
        let mut t = bare(7);
        t.world.body.players[0].lives = 1;
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0));
        for _ in 0..DEATHBOMB_WINDOW {
            assert!(t.advance(&InputFrame::empty(0)).rewound.is_none());
        }
        assert_eq!(t.world().body.players[0].life_state, crate::player::LIFE_GAMEOVER);
        assert_eq!(t.world().body.players[0].deaths, 1);
    }

    /// 落地代价从快照重算 + 符卡失格 + 残机下限 1 的边缘（快照残机 1、死分支里奖命到 2 再死）。
    #[test]
    fn landing_recomputes_the_cost_from_the_snapshot() {
        let mut t = bare(16);
        let boss = crate::world::test_support::spawn_enemy(&mut t.world, 0, 100, 1000);
        assert!(t.world.body.spell_begin_internal(0, boss, 1, 3000, 1000, 0, 100));
        t.world.body.players[0].lives = 1;
        t.ring.push(&t.world);
        while t.frame() < 40 {
            t.advance(&InputFrame::empty(0));
        }
        t.world.body.players[0].lives = 2; // 死分支里「吃到奖命」（带外写，同 plant_hit）
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0));
        let (_, cut) = die_and_rewind(&mut t);
        assert_eq!(cut.to, 40 - REWIND_DEPTH);
        let p = t.world().body.players[0];
        assert_eq!(p.life_state, LIFE_ALIVE);
        assert_eq!(p.lives, 1, "快照残机 1 − 1 → 钳 1");
        assert_eq!(p.deaths, 1);
        assert_eq!(t.world().body.spells[0].capture_ok, 0, "落地作废资格");
    }
```
- `replay_reproduces_live_checksums_including_jumps_and_rewinds`：`build` 里 `t.ring.push` 之前加 `t.world.body.players[0].lives = 200; // 死亡即遡行：给足命，免得随机走位打到 GAMEOVER`；循环次数 `0..400` → `0..1400`；删 `if st == LIFE_DEATHWINDOW { b = BTN_REWIND; } else` 分支，改为
```rust
            let try_jump = st == LIFE_ALIVE && live.frame() % 37 == 20;
            if try_jump {
                b = BTN_JUMP;
            }
            let adv = live.advance(&keys(b));
            if try_jump && live.world().body.players[0].life_state == LIFE_JUMPING {
                jumps += 1;
            }
```
（原 `jumps += 1` 行删，`let adv = live.advance(...)` 原行删，其后 `if let Some(c) = adv.rewound` 保留）；断言文案「实测」不变。
- 模块头 `use crate::player::{...}` 去掉不再用的项。

`replay.rs` 测试 `demo_run_replays_bitwise_through_bytes`：import 删 `BTN_REWIND`、`LIFE_DEATHWINDOW`，加 `LIFE_JUMPING`；`let ld = Loadout::default();` → `let ld = Loadout { lives: 200, ..Loadout::default() };`；`for _ in 0..900` → `0..1500`；循环体改为
```rust
            let st = live.world().view().players()[0].life_state;
            let mut b = match rng.rand_range(4) {
                0 => BTN_LEFT,
                1 => BTN_RIGHT,
                2 => BTN_UP,
                _ => BTN_DOWN,
            };
            let try_jump = st == LIFE_ALIVE && live.frame() % 700 == 60;
            if try_jump {
                b = BTN_JUMP;
            }
            let mut f = InputFrame::empty(live.frame());
            f.actions[0].buttons = b;
            if live.advance(&f).rewound.is_some() {
                rewinds += 1;
            }
            if try_jump && live.world().view().players()[0].life_state == LIFE_JUMPING {
                jumps += 1;
            }
```
断言 `jumps >= 3` → `jumps >= 2`（文案「demo 局里必须真的跳过（冷却 600，实测 {jumps}）」）；文档首行「进决死窗口就遡行」→「死了自动遡行」。

`input.rs` 测试：`action_bit_values_frozen` 删 `BTN_REWIND` 行、注释补「位 9 退役（旧 BTN_REWIND）」；`edge_mask_is_exactly_the_edge_actions` 期望 `BTN_BOMB | BTN_JUMP | BTN_CONTINUE`；`actions_table_matches_constants` 删 `BTN_REWIND` 行、长度 9；`max_bit_used_pinned` 文档「最高位 = 10（BTN_CONTINUE）」。

- [ ] **Step 2: 跑确认失败**

Run: `cargo test -p stg-core --lib deathwindow_expiry 2>&1 | tail -10`
Expected: 编译错（`deaths` 字段不存在）。

- [ ] **Step 3: 实现**

`player.rs`：删 `pub const LIFE_RESPAWNING: u8 = 3; ...`，原位注释 `// 3 退役（原 LIFE_RESPAWNING 场底重生，玩法刀 2026-09-14：死亡即遡行）：值不复用。`；`RESPAWN_INVULN` 注释改 `// 续关无敌帧（2 秒 @60Hz；玩法刀起唯一消费者是 try_continue）`；`REWIND_INVULN` 文档补「死亡原地继续（无 timeline 宿主）与遡行落地共用」；`PlayerState` 在 `continues` 后加
```rust
    /// 偏差值 = 本局死亡次数（玩法刀，gameplay-design §3）。纯叙事计数，不进任何战斗数值；
    /// `commit_death` 与 `rewind_landed` 各加一（前者在死分支、后者在恢复出的世界），续关不清。
    pub deaths: u8,
```
`spawn` 加 `deaths: 0,`。

`input.rs`：删 `BTN_REWIND = 9, Edge;` 及其两行文档。

`events.rs` `EVT_REWIND_REQUESTED` 文档首句改：「自机死亡请求遡行（玩法刀：相位 3 C 组 `commit_death` 在决死窗口耗尽且残机未尽时产出）」；删「World 之上没有 timeline 的宿主…决死窗口照常走完」一句，换「无 timeline 的宿主看到它就是无人认领的事实——世界已原地继续」。

`world/player.rs`：
- 文件头 `use crate::player::{...}` 删 `LIFE_RESPAWNING`、`RESPAWN_INVULN`（续关里改全路径 `crate::player::RESPAWN_INVULN`）。
- 模块文档第 6 行「决死窗口倒数 → `commit_death` → 重生 → 无敌耗尽 → Alive」→「决死窗口倒数 → `commit_death`（原地继续 + 遡行请求 / GAMEOVER）」。
- C 组 `match` 删 `LIFE_RESPAWNING => {...}` 臂。
- `commit_death` 整体替换为：
```rust
    /// 决死窗口耗尽（玩法刀 spec §4.2）：死亡即遡行。扣残机 + 偏差值 + `EVT_PLAYER_DIED`；
    /// 残机耗尽 → GAMEOVER（不遡行）；否则**原地**回 ALIVE、`REWIND_INVULN` 无敌，并发
    /// `EVT_REWIND_REQUESTED`。有 timeline 的宿主据此恢复被弹前的快照，代价在
    /// `rewind_landed` 从快照重算；无 timeline 的宿主（golden/storm/bench）到此为止 = 原地继续。
    fn commit_death(&mut self, i: usize) {
        let p = &mut self.players[i];
        p.lives = p.lives.saturating_sub(1);
        p.deaths = p.deaths.saturating_add(1);
        let (x, y, lives, hit_frame) = (p.x, p.y, p.lives, p.hit_frame);
        self.push_event(Event {
            kind: crate::events::EVT_PLAYER_DIED,
            a_index: i as u16,
            a_gen: 0,
            x,
            y,
            data: [lives as i32, 0],
        });
        if lives == 0 {
            self.players[i].life_state = LIFE_GAMEOVER;
            return;
        }
        let p = &mut self.players[i];
        p.life_state = LIFE_ALIVE;
        p.state_timer = 0;
        p.invuln = p.invuln.max(crate::player::REWIND_INVULN);
        self.push_event(Event {
            kind: crate::events::EVT_REWIND_REQUESTED,
            a_index: i as u16,
            a_gen: 0,
            x,
            y,
            data: [hit_frame as i32, 0],
        });
    }
```
- A 组删 `self.try_rewind(i);`；删 `fn try_rewind` 整个（含文档）。
- `rewind_landed` 替换为：
```rust
    /// 遡行落地（写 API，玩法刀 spec §4.3）：timeline 把世界恢复到被弹前的快照后调它。快照里的
    /// 残机/偏差值/符卡资格都是旧值，**代价全部在这里重算**：偏差值 +1、残机 −1 且下限 1（致死
    /// 与否只在死的那一刻由 `commit_death` 判，落地不再判死）、无敌取 max、active 卡失格。
    /// 落点不一定是 ALIVE（可能正在跳躍），故不断言状态。`i` 越界 → no-op + `contract_viol`（P4-b）。
    pub fn rewind_landed(&mut self, i: usize) {
        if i >= crate::MAX_PLAYERS {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            return;
        }
        let p = &mut self.players[i];
        p.deaths = p.deaths.saturating_add(1);
        p.lives = p.lives.saturating_sub(1).max(1);
        p.invuln = p.invuln.max(crate::player::REWIND_INVULN);
        self.void_spell_captures();
    }
```
- `try_continue`：删 `p.x = Fx::ZERO;`、`p.y = Fx::from_int(384);`；`p.life_state = LIFE_RESPAWNING;` → `p.life_state = LIFE_ALIVE;`；`RESPAWN_INVULN` → `crate::player::RESPAWN_INVULN`；文档「走 `commit_death` 同款重生分支（场底中心 + `RESPAWN_INVULN`）」→「原地复活 + `RESPAWN_INVULN`（玩法刀：场底重生退役）；`deaths` 不清」。

`timeline.rs` 模块文档第 5 行「世界侧只有 `try_rewind` 发请求」→「世界侧只有 `commit_death` 发请求」。

`bridge.rs`：删 `#[constant] const BTN_REWIND ...` 两行；上方注释「跳躍/遡行两个沿触发位」→「跳躍沿触发位（遡行键玩法刀退役：死亡即遡行）」。

- [ ] **Step 4: 全量 + 哨兵 + 词表指纹**

Run: `cargo test --workspace 2>&1 | grep -E '^test result|FAILED|panicked'`
Expected: 全绿，除 `vocab_hash_pinned`（按实测改值，注释「玩法刀删 BTN_REWIND=9 → 0x...」）与可能的尺寸哨兵（追加账目「玩法刀 Task 5：`deaths: u8`」）。`timeline` 回放测试若 `rewinds >= 2` 不满足，把 1400 帧加到 2000（不要改弹阵）。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
git add -A crates && git commit -m "feat(core): 玩法刀 T5——死亡即遡行（commit_death 原地继续+请求，rewind_landed 从快照重算残机/偏差值/失格）+ deaths；BTN_REWIND 位 9 与 LIFE_RESPAWNING 退役

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: ENGINE_VER 20、金向量、生成物、全闸门

**Files:**
- Modify: `crates/stg-core/src/lib.rs:186-193`
- Modify: `crates/stg-core/src/step.rs`（`engine_ver_anchored`；尺寸哨兵账目核对）
- Regenerate: `docs/ecl-lang/7-reference.md`（生成段）、`editors/vscode/stg-ecl/ecl-meta.json`

- [ ] **Step 1: 改锚点测试（先红）**

`step.rs::engine_ver_anchored`：`19,` → `20,`，消息串开头插入：
```
"bump 必须是有意识决定(评审 + 改本测试)——19→20：玩法刀(2026-09-14)。\
 PlayerState 删 bomb_phase/bomb_timer/time_stops、加 jump_cd u16/deaths u8(存档 wire format 变);\
 碰撞矩阵新增行 8 ROW_STOP_TOUCH(停止冻结中触碰消弹);号表 513 add_time_stops 退役;\
 输入词表退役位 7 BTN_TIMESTOP / 位 9 BTN_REWIND(vocab_hash 变);生命态 LIFE_RESPAWNING=3 退役;\
 WorldTables 删 CharacterCfg.bomb(TABLE_VERSION 5,content_hash 变);回放头 LOG_FILE_VER 2。\
 ——前一次 18→19：壳子刀(2026-09-11)。\
```
（原「——18→19：壳子刀…」起的旧文保留在其后，删掉原首句里重复的「bump 必须是有意识决定(评审 + 改本测试)——」。）

Run: `cargo test -p stg-core --lib engine_ver_anchored 2>&1 | tail -3` → Expected: FAIL（19 != 20）。

- [ ] **Step 2: bump**

`lib.rs`：`pub const ENGINE_VER: u32 = 20;`，其上文档追加：
```rust
///
/// **19 → 20**（玩法刀，2026-09-14）：**布局 + 碰撞矩阵 + 号表 + 词表 + 表格式五重变更**。
/// ① `PlayerState` 删 `bomb_phase: u8`/`bomb_timer: u16`/`time_stops: u8`、加 `jump_cd: u16`/
/// `deaths: u8` ⇒ 存档 wire format 变；② 碰撞矩阵行 8 `ROW_STOP_TOUCH`（相位 6/7/9 冻结分支）；
/// ③ syscall 513 `add_time_stops` 退役（号不复用）、`add_bombs` 上钳 5；④ 输入词表退役位 7/9、
/// 生命态退役 3（`LIFE_RESPAWNING`），死亡改为原地继续 + `EVT_REWIND_REQUESTED`；⑤ `WorldTables`
/// 删 `CharacterCfg.bomb`（`TABLE_VERSION` 5）⇒ 表 `content_hash` 变；回放头 `LOG_FILE_VER` 2。
/// **金向量预期改变**（布局自帧 0 起进哈希，且风铃卡场景自机若死亡则行为改变）；实测为准。
```

- [ ] **Step 3: 生成物 + 金向量 + 闸门**

```bash
cargo run -q -p stg-harness -- gen-ecl-meta
git diff --stat docs/ecl-lang/7-reference.md editors/vscode/stg-ecl/ecl-meta.json
cargo run -q -p stg-harness -- verify-tables
SP=/tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/0ee127e4-0082-400e-80c4-07caafa7cc2c/scratchpad
cargo run -q -p stg-harness -- golden --out $SP/golden_new.txt && md5sum $SP/golden_new.txt
diff <(head -5 $SP/golden_base.txt) <(head -5 $SP/golden_new.txt) | head
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E '^test result|FAILED|panicked'
cargo run -q --release -p stg-harness -- storm 2>&1 | tail -3
```
Expected: gen-ecl-meta 只删 `add_time_stops` 一条、改 `add_bombs` doc；verify-tables ✔；金向量 md5 ≠ `978522fd…`（记下新 md5，首差帧应为帧 0——布局变化）；fmt/clippy 无输出；test 全绿；storm 通过。

尺寸哨兵账目核对：读 `world_size_sentinel_guards_copy_into_field_list` 末尾三段玩法刀账目，确认 T1/T4/T5 各段数字与当前实测一致、三段首尾相接。

- [ ] **Step 4: 提交**

```bash
git add -A crates docs/ecl-lang/7-reference.md editors/vscode/stg-ecl/ecl-meta.json
git commit -m "chore(core): 玩法刀 T6——ENGINE_VER 19→20 + ecl 元数据重生成；金向量 md5 <新值>

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```
（`<新值>` 用 Step 3 实测 md5 替换后再执行。）

---

### Task 7: 桥——常量与 HUD 两键 + 桥级冒烟

**Files:**
- Modify: `crates/stg-godot/src/bridge.rs`（常量、`hud_player`）
- Modify: `crates/stg-godot/smoke/smoke.gd:183-186, 262-302, 317-323`

**Interfaces:**
- Produces（GDScript 可见）：`WorldBridge.JUMP_COOLDOWN`、`WorldBridge.STOP_STOCK_MAX`、`hud_player()["jump_cd"]`、`hud_player()["deaths"]`。

- [ ] **Step 1: 冒烟先改成新口径（先红）**

`smoke.gd`：
- `if hp.get("bombs", -1) != 3: fail("hud_player bombs"); return` → `!= 2`，并加一行 `if hp.get("deaths", -1) != 0: fail("hud_player deaths"); return` 与 `if hp.get("jump_cd", -1) != 0: fail("hud_player jump_cd"); return`。
- ① 跳躍段末尾 `if int(b.hud_player()["life_state"]) != 1: fail("time: 第 N 帧应回 ALIVE"); return` 之后加：
```gdscript
	if int(b.hud_player()["jump_cd"]) != WorldBridge.JUMP_COOLDOWN: fail("time: 落地应写满冷却"); return
	if b.step_frame(WorldBridge.BTN_JUMP) != -1 or int(b.hud_player()["life_state"]) != 1: fail("time: 冷却中按跳躍应无效"); return
```
- ③ 遡行段：注释改「先上到 y≈300，再向左撞弹流；中弹后空跑到决死窗口耗尽 → step_frame 返回落点」；从 `var g0: int = b.frame()` 到 `if int(b.hud_player()["invuln"]) <= 0: ...` 替换为：
```gdscript
	var lives_before: int = int(b.hud_player()["lives"])
	var landed := -1
	var g0 := -1
	var waited_w := 0
	while landed < 0 and waited_w <= 20:
		g0 = b.frame()
		landed = b.step_frame(0); waited_w += 1
	var expect_to := maxi(hit_frame - WorldBridge.REWIND_DEPTH, ring_oldest)
	if landed != expect_to: fail("time: 遡行落点应为 %d,得 %d(hit_frame %d)" % [expect_to, landed, hit_frame]); return
	if b.frame() != expect_to: fail("time: frame() 应回到落点"); return
	if int(b.hud_player()["life_state"]) != 1: fail("time: 落地应为 ALIVE"); return
	if int(b.hud_player()["invuln"]) <= 0: fail("time: 落地应有无敌帧"); return
	if int(b.hud_player()["deaths"]) != 1: fail("time: 偏差值应为 1"); return
	if int(b.hud_player()["lives"]) != lives_before - 1: fail("time: 落地残机应 −1"); return
```
  其后 `view_ring(g0 + 1)` 两行改为读 `g0`（窗口最后一帧，仍是 DEATHWINDOW）：
```gdscript
	if not b.view_ring(g0): fail("time: view_ring(被丢弃的窗口末帧)"); return
	if int(b.hud_player()["life_state"]) != WorldBridge.LIFE_DEATHWINDOW: fail("time: 视图帧应是决死窗口那一帧"); return
```
  注意：`lives_before` 取值须在决死窗口内（进入 while 前 `hit_frame` 已测到、窗口未耗尽），故放在 `if hit_frame < 0: fail...` 之后。
- 续关段注释「RESPAWNING」→「ALIVE」，`if int(hp2["life_state"]) != 3: fail("shell: 续关后应 RESPAWNING(3),得 %d" ...` → `!= 1` / `"shell: 续关后应 ALIVE(1),得 %d"`。

Run: `bash crates/stg-godot/smoke/run-smoke.sh 2>&1 | tail -5`
Expected: SMOKE FAIL（`hud_player deaths` 键不存在 / `WorldBridge.JUMP_COOLDOWN` 未定义）。

- [ ] **Step 2: 实现桥**

`bridge.rs` 常量段（`JUMP_FRAMES` 之后）加：
```rust
    // 玩法刀(2026-09-14):跳躍冷却 + 停止库存上限(HUD 冷却条/库存显示用)。
    #[constant]
    const JUMP_COOLDOWN: i64 = stg_core::player::JUMP_COOLDOWN as i64;
    #[constant]
    const STOP_STOCK_MAX: i64 = stg_core::player::STOP_STOCK_MAX as i64;
```
`hud_player` 在 `continues` 之后加：
```rust
        d.set("jump_cd", p.jump_cd as i64);
        d.set("deaths", p.deaths as i64);
```

- [ ] **Step 3: 跑**

```bash
cargo build -p stg-godot && cargo test -p stg-godot 2>&1 | grep -E '^test result|FAILED'
bash crates/stg-godot/smoke/run-smoke.sh 2>&1 | tail -5
```
Expected: 桥单测全绿；`SMOKE OK`。若遡行段 `waited_w` 超 20 仍无落地：打印 `hud_player()` 定位（决死窗口 8 帧，不应超）。

- [ ] **Step 4: 提交**

```bash
cargo fmt --all && git add -A crates/stg-godot && git commit -m "feat(godot-bridge): 玩法刀 T7——JUMP_COOLDOWN/STOP_STOCK_MAX 常量 + hud_player.jump_cd/deaths；桥冒烟改死亡遡行/续关 ALIVE/默认库存 2

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: 壳——键位、HUD、観測窗口 3 s、提示文案 + 工程冒烟 + 有头目验

**Files:**
- Modify: `godot/scripts/input.gd`
- Modify: `godot/scripts/hud.gd`
- Modify: `godot/scripts/play.gd:51-60, 65-68, 337-346, 378-385`

- [ ] **Step 1: 改 `input.gd`**

`KEYS` 删 `"stg_timestop": KEY_D,` 与 `"stg_rewind": KEY_V,`；`mask()` 删那两行；文件头注释改为：
```gdscript
## InputMap 代码注册(免手写序列化;物理键:方向键 + Z 射 X 停止 Shift 低速 C 観測/跳躍)。
## 位掩码零翻译:WorldBridge.BTN_* 即 stg-core 动作位。玩法刀(2026-09-14):X = 停止(BTN_BOMB,
## 时停+触碰消弹合一),D/V 两键退场(遡行改为死亡自动触发)。**跳躍位不在 mask() 里**:
## 観測→跳躍的两段协议归 play.gd,只在第二下按 C 时注入一帧 BTN_JUMP。
```

- [ ] **Step 2: 改 `hud.gd`**

成员加 `var deaths_l: Label`、`var jump_bg: ColorRect`、`var jump_bar: ColorRect`；`_ready` 里面板行改为
```gdscript
	score_l = _row(panel); lives_l = _row(panel); bombs_l = _row(panel)
	power_l = _row(panel); graze_l = _row(panel); deaths_l = _row(panel); bgm_l = _row(panel)
	# 跳躍冷却条(玩法刀):满 = 可跳;只画条不写数字(gameplay-design §2)。
	jump_bg = ColorRect.new()
	jump_bg.custom_minimum_size = Vector2(120, 4)
	jump_bg.color = Color(1, 1, 1, 0.15)
	panel.add_child(jump_bg)
	jump_bar = ColorRect.new()
	jump_bar.size = Vector2(120, 4)
	jump_bar.color = Color(0.55, 0.8, 1.0)
	jump_bg.add_child(jump_bar)
```
文件头注释「分/残机/bomb/power/graze + 曲名」→「分/残机/停止/power/graze/偏差 + 冷却条 + 曲名」；`refresh` 里：
```gdscript
	bombs_l.text = "Stop   %d (%d)" % [int(p["bombs"]), int(p["bomb_pieces"])]
	deaths_l.text = "偏差   %d" % int(p.get("deaths", 0))
	var cd := int(p.get("jump_cd", 0))
	jump_bar.size.x = 120.0 * (1.0 - float(cd) / float(WorldBridge.JUMP_COOLDOWN))
```

- [ ] **Step 3: 改 `play.gd`**

- `const OBSERVE_WINDOW := 60` → `180`；其上注释「窗口 OBSERVE_WINDOW tick 内」补「（玩法刀：3 s）」。
- `_update_time_hint`：
```gdscript
	if observing:
		var cd := int(bridge.hud_player().get("jump_cd", 0))
		var tail := "冷却中" if cd > 0 else "C 跳躍"
		hud.set_time_hint("観測 %d  (%s)" % [_observe_left, tail])
	elif st == WorldBridge.LIFE_JUMPING:
		hud.set_time_hint("跳躍")
	elif st == WorldBridge.LIFE_DEATHWINDOW:
		hud.set_time_hint("X 停止")
	else:
		hud.set_time_hint("")
```
- 目验常量：`SHOT_FRAMES` 里 `201: "bomb_t1", 206: "bomb_t6", 214: "bomb_t14"` → `201: "stop_t1", 206: "stop_t6", 214: "stop_t14"`；`const SHOT_BOMB_FRAME := 200` → `const SHOT_STOP_FRAME := 200`；`_scripted_mask` 里同名替换，注释「按一帧 bomb」→「按一帧停止」；`--shots` 模式说明注释里「第 200 帧放 bomb」两处 →「第 200 帧按停止」。

- [ ] **Step 4: 工程冒烟**

```bash
cargo build -p stg-godot
bash godot/smoke/run-smoke.sh 2>&1 | tail -8
```
Expected: `SMOKE OK`（観測 100 帧开、130 帧跳在 180 窗口内；流程冒烟不依赖停止/遡行键）。GDScript 解析错（如仍引用 `BTN_TIMESTOP`）会让 headless 报 `SCRIPT ERROR`——`grep -rn 'BTN_TIMESTOP\|BTN_REWIND\|stg_timestop\|stg_rewind' godot` 应无输出。

- [ ] **Step 5: 有头目验（VNC `:2` 已在跑）**

```bash
SHOTS=/tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/0ee127e4-0082-400e-80c4-07caafa7cc2c/scratchpad/shots
mkdir -p $SHOTS
STG_SHOTS_DIR=$SHOTS DISPLAY=:2 LIBGL_ALWAYS_SOFTWARE=1 timeout 300 godot --rendering-driver opengl3 --path godot -- --shots 2>&1 | tail -5
ls $SHOTS
```
用 Read 查看 `stop_t6_f206.png`（冻结画面 + HUD `Stop` 行比开局少 1、`偏差 0`）、`jump_f361.png` 之后一张（HUD 冷却条从空开始）。**口径说明**：脚本化输入不保证触碰到弹、不保证死亡——触碰消弹与死亡倒放的正确性由 Task 3/5 单测与 Task 7 桥冒烟押，目验只确认 HUD 三件与停止画面渲染正常；如需人眼验触碰/倒放，交用户经 noVNC 手玩（`godot --path godot`）。

- [ ] **Step 6: 提交**

```bash
git add -A godot && git commit -m "feat(godot): 玩法刀 T8——X 停止/D·V 退场、HUD 停止·偏差·冷却条、観測窗口 3 s、决死提示 X 停止、目验帧名 stop_*

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: 文档收口

**Files:**
- Modify: `docs/ecl-ops.md:208`、`docs/ecl-lang/6-spell-and-stage.md:~336`、`docs/render-contract.md:53`（+ `vanished` 冻结口径段）、`docs/gameplay-design.md` §2/§10、`docs/follow-ups.md`（F19、C12⑤、头部追记）、`stg-world-design.md:228,462,479,614`、`CLAUDE.md:226`、`docs/superpowers/specs/2026-09-14-gameplay-tools-design.md` §10、`PROGRESS.md`

- [ ] **Step 1: 逐文件改**

- `docs/ecl-ops.md` 513 行改为 `| 513 | ~~`add_time_stops`~~ **退役**（玩法刀 2026-09-14，号不复用） | — | — |`；511 行语义列补「钳 `[0, STOP_STOCK_MAX=5]`（玩法刀起，语义 = 停止库存）」。
- `docs/ecl-lang/6-spell-and-stage.md` 约 336 行讲 bomb/`BombCfg` 的那段改写为：「玩家的 X 是**停止**（时停 180 帧 + 触碰消弹，库存 = `add_bombs` 管的那份，上限 5）；引擎内已无 bomb 作用区。脚本侧能做的只有 `add_bombs(n)` 发库存、`clear_bullets()` 清场。」
- `docs/render-contract.md`：53 行自机表现字段表删 `bomb_phase`；`vanished` 小节（§3.6）补一句「停止冻结期间只出现 `VANISH_CLEARED`（触碰消弹当帧回收）；寿尽/越界留到解冻」；§8 目验帧名若出现 `bomb_t*` 改 `stop_t*`（`grep -n bomb_t docs/render-contract.md`）。
- `docs/gameplay-design.md` §2 第一条「按住 C，…松开取消」改为「按 C 进入観測（3 s 窗口，180 tick），屏幕上叠一层影子弹幕…窗口内再按 C = 跳躍；窗口到时自动退出。」§4 键位表 C 行改「観測（按）→ 跳躍（窗内再按）」；§10 标题下加一行「**落地状态（2026-09-14 玩法刀）**：1–4、6、7 已落地（spec `docs/superpowers/specs/2026-09-14-gameplay-tools-design.md`）；5（harness 探针）待探针刀。」
- `docs/follow-ups.md`：删 **F19** 整条；C12⑤ 里「`attract_all_items`」一项删（随 bomb 退役删了函数）；头部维护段顶上加一段：「> **玩法刀（2026-09-14）销 1 条**：**F19**（时停键占 D——停止合并后 X = 停止，D/V 退场）。C12⑤ 删 `attract_all_items` 一项（函数随 bomb 退役删除）。」
- `stg-world-design.md`：228、462、479、614 四处 bomb 描述各加行内修订注「〔2026-09-14 玩法刀：bomb 退役，X = 停止（时停 + 触碰消弹，碰撞行 8），`bomb_phase/bomb_timer`/`BombCfg` 已删；FieldPool 现仅 ECL `clear_bullets` 使用〕」；D6 生命态若列出 RESPAWNING（`grep -n RESPAWN stg-world-design.md`），同样行内注「〔玩法刀：退役，死亡即遡行〕」。
- `CLAUDE.md` 226 行「`BTN_JUMP`/`BTN_REWIND`」→「`BTN_JUMP`（`BTN_REWIND` 玩法刀退役）」；仓库结构 `src/world/` 行「player(相1+3，shottype 表驱动发弹)」→「player(相1+3，shottype 发弹 / 停止 / 跳躍冷却 / 死亡即遡行)」。
- spec §10「（收口时填）」替换为本刀实测偏差（至少：金向量新 md5；各 Task 尺寸哨兵是否响；`TableLoadError::BadDiscriminant` 处置；demo 回放闸帧数/跳躍次数是否调整过）。

- [ ] **Step 2: PROGRESS**

「现在」段重写（≤10 行）：位置 = 玩法刀落地（`ENGINE_VER` 20、金向量 md5、测试计数实测）；下一步 = 第 1 关内容刀（§6）→ harness 探针刀（`probe-jump` / `replay --cut-gaps`）→ 验证 V1–V7；点阵委托前 F24。里程碑史表顶加一行：
`| 2026-09-14 | **玩法刀（停止 / 跳躍冷却 / 死亡即遡行）** | 核：... ；桥：...；壳：...；闸门全绿（...）。 |`（按实测填满，一行）。

- [ ] **Step 3: 最终闸门 + 提交**

```bash
cargo test --workspace 2>&1 | grep -E '^test result|FAILED'   # ecl-lang 围栏被真编译，文档改动须仍绿
git add -A docs CLAUDE.md stg-world-design.md PROGRESS.md
git commit -m "docs: 玩法刀收口——ecl-ops/手册/render-contract/gameplay-design/world-design/CLAUDE.md 同步，销 F19，PROGRESS 史一行

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 4: 交还用户**：汇报测试计数、金向量 md5、目验截图结论、未做（探针刀），询问是否合并 `feat/gameplay` 进 main（走 superpowers:finishing-a-development-branch）。
