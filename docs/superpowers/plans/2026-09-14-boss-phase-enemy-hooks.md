# boss 换段与敌人钩子刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> **本仓约定**：执行 inline，任务间自检从简（memory `skip-heavy-superpowers-flow`）；TDD 与 CLAUDE.md 自检清单照做。

**Goal:** 给 `.ecl` 补上第 1 关必需、现有能力合成不了的五件：非符段 + 超时钉血 + 结束方式读口、`spawn_enemy` 带参、敌判定写口、清场 + 自身敌号、半径清弹 / 不给星清弹。

**Architecture:** 全部落在既有机构上扩展——符卡计器加一个 flag 位并把结算改为按「结束方式」分派；敌池 `flags: u8` 用两个空位；清弹区加一个标志位；syscall 号表按百分区制新增 7 号并原地改 210 调用约定；编译器新增 7 条内建 + 1 条糖 + 1 个 `$` 变量，`spawn_enemy` 的 task 位接受 `sub(实参…)`。整刀 bump 一次 `ENGINE_VER`（20→21）。

**Tech Stack:** Rust 1.94（workspace：`stg-core` / `stg-ecl-compiler` / `stg-harness` / `stg-godot`），GDScript（`godot/scripts/hud.gd`）。

**Spec:** `docs/superpowers/specs/2026-09-14-boss-phase-enemy-hooks-design.md`

## Global Constraints

- 分支 `boss-phase-enemy-hooks`；每个 Task 末尾提交一次；提交信息结尾附
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>` 与 `Claude-Session: https://claude.ai/code/session_01AGS9HJDeGZyoRXPArbESZH`。
- **撤回临时/变异代码只能用编辑，禁止 `git checkout -- <file>`**（memory：本仓被咬过两次）。
- `stg-core` 断层线以下：禁浮点、禁时钟、禁宿主 RNG、禁无序容器；遍历按池索引升序。
- P4：调用方违约 → `diag.contract_viol += 1` + `last_status = STATUS_BAD_ARGS`（悬垂句柄用 `STATUS_STALE_HANDLE`）+ 整条 no-op，不 Fault；owner 类别不符 → `Err(FAULT_BAD_OP)`；栈不够 → `Err(FAULT_STACK)`。
- syscall 号（冻结面）：`026 self_enemy` · `131 spell_result` · `210 spawn_enemy`（调用约定变更）· `440 set_invuln` · `441 set_hitbox` · `442 set_hurtbox` · `443 set_enemy_flag` · `531 kill_all_enemies` · `541 clear_bullets_at`。
- 新事件 `EVT_PHASE_ENDED = 12`。新位：`SPELL_NONSPELL = 1 << 2`、`ENEMY_NO_BODY = 1 << 1`、`ENEMY_KILLALL_EXEMPT = 1 << 2`、`FIELD_NO_STAR = 1 << 2`。
- 结束方式：`SPELL_END_HP = 1` / `SPELL_END_TIMEOUT = 2` / `SPELL_END_MANUAL = 3`；清场模式：`KILL_SILENT = 0` / `KILL_DIE = 1`。
- `ENGINE_VER` 只在 Task 6 bump 一次（20→21）。
- 常用命令：`cargo test -p stg-core <filter>`、`cargo test -p stg-ecl-compiler <filter>`、`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。

---

### Task 1: 符卡结算按结束方式分派——非符段 / 超时钉血 / 超时清弹不给星 / `spell_last_result`

**Files:**
- Modify: `crates/stg-core/src/spell.rs`（新常量、`settle_spells` 两个调用点、测试）
- Modify: `crates/stg-core/src/world.rs`（`WorldBody` 新字段、`spell_begin_internal`、`settle_one_spell`、`spell_end_by_owner`）
- Modify: `crates/stg-core/src/step.rs`（`copy_into` + 测试）
- Modify: `crates/stg-core/src/field.rs`（`FIELD_NO_STAR`、`fullscreen_clear_field_no_star`）
- Modify: `crates/stg-core/src/world/settle.rs`（趟一跳过转星星 + 测试）
- Modify: `crates/stg-core/src/events.rs`（`EVT_PHASE_ENDED`）
- Modify: `crates/stg-core/src/consts.rs`（注入 6 个 `SPELL_*`）

**Interfaces:**
- Produces: `crate::spell::{SPELL_NONSPELL, SPELL_END_HP, SPELL_END_TIMEOUT, SPELL_END_MANUAL}: u8`；
  `WorldBody::spell_last_result: [u8; MAX_BOSSES]`（`pub(crate)`）；
  `WorldBody::settle_one_spell(&mut self, slot: usize, cause: u8)`；
  `crate::field::FIELD_NO_STAR: u8`、`crate::field::fullscreen_clear_field_no_star() -> FieldInit`；
  `crate::events::EVT_PHASE_ENDED: u8 = 12`。

- [ ] **Step 1: 写失败测试（spell.rs 测试模块末尾追加）**

```rust
    /// 超时钉血（spec §3.2 ①）：普通卡 / 耐久卡 / 非符段三种超时都把 hp 钉到血线，
    /// 下一段 `hp_start` 从血线起（剩血不漏段）。
    #[test]
    fn timeout_pins_hp_to_threshold_for_all_three_kinds() {
        for flags in [0u8, SPELL_SURVIVAL, SPELL_NONSPELL] {
            let (mut w, boss) = world_with_boss(1000);
            assert!(w.body.spell_begin_internal(0, boss, 3, 1, 1000, flags, 300));
            w.body.settle_spells(&crate::tables::TABLES_V0);
            w.body.settle_spells(&crate::tables::TABLES_V0);
            let i = w.body.enemies.get(boss).unwrap();
            assert_eq!(w.body.enemies.hp[i], 300, "flags={flags}：超时钉到血线");
            assert!(w.body.spell_begin_internal(0, boss, 4, 60, 1000, 0, 0));
            assert_eq!(w.body.spells[0].hp_start, 300, "flags={flags}：下一段从血线起");
        }
    }

    /// 钉血只在超时路径、且是 `min` 不是赋值：HP 路径结算时 hp 已低于血线也不被抬回血线。
    #[test]
    fn hp_break_does_not_raise_hp_to_threshold() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 3, 60, 1000, 0, 300));
        let i = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[i] = 200;
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spells[0].active, 0);
        assert_eq!(w.body.enemies.hp[i], 200);
    }

    /// 超时结算铺的全屏清弹区带 FIELD_NO_STAR；HP 路径不带（spec §3.2 ③）。
    #[test]
    fn timeout_clear_field_has_no_star_bit_hp_break_does_not() {
        use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 3, 1, 0, 0, 0));
        w.body.settle_spells(&crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        let f = w.body.fields.iter_alive().next().expect("超时结算应铺清弹区");
        assert_eq!(w.body.fields.flags[f], FIELD_CLEAR_BULLETS | FIELD_NO_STAR);

        let (mut w2, boss2) = world_with_boss(1000);
        assert!(w2.body.spell_begin_internal(0, boss2, 3, 60, 0, 0, 300));
        let i = w2.body.enemies.get(boss2).unwrap();
        w2.body.enemies.hp[i] = 300;
        w2.body.settle_spells(&crate::tables::TABLES_V0);
        let f2 = w2.body.fields.iter_alive().next().expect("HP 结算应铺清弹区");
        assert_eq!(w2.body.fields.flags[f2], FIELD_CLEAR_BULLETS);
    }

    /// 非符段（spec §3.1/§3.2 ②）：不宣言、bonus 恒 0、SURVIVAL 位被忽略、不付分、
    /// 不发 CAPTURED/FAILED/REQ_SPELL_RESULT，结束发 EVT_PHASE_ENDED[spell_id, cause]。
    #[test]
    fn nonspell_skips_declare_bonus_and_result_and_emits_phase_ended() {
        use crate::events::{EVT_PHASE_ENDED, EVT_SPELL_DECLARED};
        let (mut w, boss) = world_with_boss(1000);
        let score0 = w.body.players[0].score;
        assert!(w.body.spell_begin_internal(
            0,
            boss,
            9,
            1,
            5000,
            SPELL_NONSPELL | SPELL_SURVIVAL,
            300
        ));
        assert_eq!(w.body.spells[0].bonus_now, 0);
        assert!(
            (0..w.body.frame_events_len as usize)
                .all(|k| w.body.frame_events[k].kind != EVT_SPELL_DECLARED)
        );
        assert!(
            w.body
                .take_requests()
                .iter()
                .all(|r| r.id != crate::consts::REQ_SPELL_DECLARE)
        );
        w.body.settle_spells(&crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        let ev = last_event(&w);
        assert_eq!(ev.kind, EVT_PHASE_ENDED);
        assert_eq!(ev.data, [9, SPELL_END_TIMEOUT as i32]);
        assert_eq!(w.body.players[0].score, score0);
        assert!(
            w.body
                .take_requests()
                .iter()
                .all(|r| r.id != crate::consts::REQ_SPELL_RESULT)
        );
    }

    /// `spell_last_result`（spec §3.3）：初值 0；三种结束方式各记 1/2/3；
    /// 新 begin 不清；只动本槽。
    #[test]
    fn spell_last_result_records_cause_and_survives_next_begin() {
        let (mut w, boss) = world_with_boss(1000);
        assert_eq!(w.body.spell_last_result, [0; crate::boss::MAX_BOSSES]);
        assert!(w.body.spell_begin_internal(0, boss, 1, 60, 0, 0, 900));
        let i = w.body.enemies.get(boss).unwrap();
        w.body.enemies.hp[i] = 900;
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_HP);

        assert!(w.body.spell_begin_internal(0, boss, 2, 1, 0, 0, 800));
        assert_eq!(w.body.spell_last_result[0], SPELL_END_HP, "begin 不清读口");
        w.body.settle_spells(&crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_TIMEOUT);

        assert!(w.body.spell_begin_internal(0, boss, 3, 60, 0, 0, 0));
        w.body.spell_end_by_owner(boss);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_MANUAL);
        assert_eq!(w.body.spell_last_result[1], 0, "别的槽不动");
    }
```

settle.rs 测试模块追加：

```rust
    /// FIELD_NO_STAR（spec §6）：弹照清、清弹事件照计，但不转星星。
    #[test]
    fn no_star_field_clears_without_spawning_stars() {
        use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 40, FIELD_CLEAR_BULLETS | FIELD_NO_STAR, 1);
        let b = bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        let bi = w.body.bullets.get(b).unwrap();
        assert_ne!(w.body.bullets.flags[bi] & crate::bullets::BULLET_CLEARED, 0);
        assert_eq!(w.body.items.iter_alive().count(), 0, "不转星星");
        assert_eq!(w.body.frame_events[0].kind, EVT_FIELD_CLEARED);
    }
```

step.rs 测试模块追加：

```rust
    /// 新字段 `spell_last_result`（2 B，可能被对齐吞掉、尺寸哨兵不响——D20）：
    /// 直测它进校验和、随 copy_into 往返。
    #[test]
    fn spell_last_result_rides_copy_into_and_checksum() {
        let mut a = World::new(1);
        let mut b = World::new(1);
        let c0 = a.checksum();
        a.body.spell_last_result[1] = crate::spell::SPELL_END_TIMEOUT;
        assert_ne!(a.checksum(), c0, "新字段必须进校验和");
        a.copy_into(&mut b);
        assert_eq!(b.body.spell_last_result, a.body.spell_last_result);
        assert_eq!(b.checksum(), a.checksum());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core spell_last_result nonspell timeout_pins hp_break_does_not no_star_field timeout_clear_field 2>&1 | tail -20`
Expected: 编译失败（`SPELL_NONSPELL`/`spell_last_result`/`FIELD_NO_STAR` 未定义）。

- [ ] **Step 3: 实现**

`spell.rs`（常量区，`SPELL_NO_CLEAR` 之后）：

```rust
/// 非符段（boss 换段刀 2026-09-14 spec §3.1）：计时/血线/模式随段/血条照旧；不宣言、bonus 恒 0、
/// SURVIVAL 位忽略；结算不付分、不发 CAPTURED/FAILED/REQ_SPELL_RESULT，改发 `EVT_PHASE_ENDED`。
pub const SPELL_NONSPELL: u8 = 1 << 2;

/// 结束方式（`spell_last_result` 取值 / `EVT_PHASE_ENDED.data[1]`；0 = 该槽还没结束过）。
pub const SPELL_END_HP: u8 = 1;
pub const SPELL_END_TIMEOUT: u8 = 2;
pub const SPELL_END_MANUAL: u8 = 3;
```

`settle_spells` 步骤 3/4 的调用改为（captured/reason 的推导整体搬进 `settle_one_spell`）：

```rust
            if hp_break {
                self.settle_one_spell(slot, SPELL_END_HP);
                continue;
            }
            // 4. 超时判定（captured 口径在 settle_one_spell 内按 flags 推导）。
            if s.frames_left == 0 {
                self.settle_one_spell(slot, SPELL_END_TIMEOUT);
                continue;
            }
```

`world.rs` `WorldBody` 字段（紧跟 `spell_seq` 之后）：

```rust
    /// 每槽最近一次结算的结束方式（boss 换段刀 spec §3.3）：`SPELL_END_*`，0 = 该槽还没结束过。
    /// 只由 `settle_one_spell` 写，`spell_begin` 不清；`spell_result` syscall（131）读。
    /// 全零初始化合法（`World::new` 走 `alloc_zeroed`）。
    pub(crate) spell_last_result: [u8; crate::boss::MAX_BOSSES],
```

`spell_begin_internal`：在 `let hp_start = ...` 之前加

```rust
        let nonspell = flags & crate::spell::SPELL_NONSPELL != 0;
        // 非符段 bonus 恒 0（传入值忽略、不计违约，spec §3.1）。
        let bonus0 = if nonspell { 0 } else { bonus0 };
```

并把 `push_event(EVT_SPELL_DECLARED)` 与 `emit_req(REQ_SPELL_DECLARE)` 两段包进 `if !nonspell { ... }`。

`settle_one_spell` 整体替换为：

```rust
    /// 符卡/非符段结算原子包（spec 2026-07-24 §4 + boss 换段刀 spec §3.2）。`cause` = `SPELL_END_*`。
    /// 顺序：① 超时钉血 → ② 付分与事件（普通卡按 captured 规则；非符段只发 EVT_PHASE_ENDED）
    /// → ③ 自动清弹（超时不给星）→ ④ 写 `spell_last_result` → ⑤ 槽与公告板清零。
    pub(crate) fn settle_one_spell(&mut self, slot: usize, cause: u8) {
        use crate::spell::{
            SPELL_END_TIMEOUT, SPELL_FAIL_CAPTURE_LOST, SPELL_FAIL_TIMEOUT, SPELL_NO_CLEAR,
            SPELL_NONSPELL, SPELL_SURVIVAL,
        };
        let s = self.spells[slot];
        let boss = EnemyHandle {
            index: s.boss_index,
            generation: s.boss_gen,
        };
        // ① 超时钉血：绑定 boss 仍在且未在死 → hp = min(hp, 血线)（剩血不漏进下一段）。
        if cause == SPELL_END_TIMEOUT
            && let Some(i) = self.enemies.get(boss)
            && self.enemies.flags[i] & crate::enemy::ENEMY_DYING == 0
        {
            self.enemies.hp[i] = self.enemies.hp[i].min(s.hp_threshold);
        }
        let (x, y) = match self.enemies.get(boss) {
            Some(i) => (self.enemies.x[i], self.enemies.y[i]),
            None => (Fx::ZERO, Fx::ZERO),
        };
        if s.flags & SPELL_NONSPELL != 0 {
            // ② 非符段：不付分、不发 CAPTURED/FAILED/REQ_SPELL_RESULT。
            self.push_event(Event {
                kind: crate::events::EVT_PHASE_ENDED,
                a_index: s.boss_index,
                a_gen: s.boss_gen,
                x,
                y,
                data: [s.spell_id as i32, cause as i32],
            });
        } else {
            // ② 普通卡：captured 规则沿用 spec 2026-07-24 §4 结束矩阵——普通卡超时恒 FAILED(超时)，
            // 其余（HP / 耐久卡超时 / 手动）看资格。
            let (captured, reason) = if cause == SPELL_END_TIMEOUT && s.flags & SPELL_SURVIVAL == 0 {
                (false, SPELL_FAIL_TIMEOUT)
            } else if s.capture_ok != 0 {
                (true, 0)
            } else {
                (false, SPELL_FAIL_CAPTURE_LOST)
            };
            let paid = if captured { s.bonus_now } else { 0 };
            if captured {
                self.players[0].score += paid as u64;
            }
            let kind = if captured {
                crate::events::EVT_SPELL_CAPTURED
            } else {
                crate::events::EVT_SPELL_FAILED
            };
            self.push_event(Event {
                kind,
                a_index: s.boss_index,
                a_gen: s.boss_gen,
                x,
                y,
                data: [
                    s.spell_id as i32,
                    if captured { paid as i32 } else { reason },
                ],
            });
            self.emit_req(
                crate::consts::REQ_SPELL_RESULT,
                [s.spell_id as i32, captured as i32, paid as i32, reason, 0, 0],
            );
        }
        // ③ 自动清弹：超时路径不给星（对齐 ZUN 超时走 etClear）。
        if s.flags & SPELL_NO_CLEAR == 0 {
            let field = if cause == SPELL_END_TIMEOUT {
                crate::field::fullscreen_clear_field_no_star()
            } else {
                crate::field::fullscreen_clear_field()
            };
            self.create_field(field);
        }
        // ④ 读口。
        self.spell_last_result[slot] = cause;
        // ⑤ 槽与公告板清零（B16②）。
        self.spells[slot] = crate::spell::SpellSlot::default();
        self.boss_ui[slot] = crate::boss::BossUiSlot::default();
    }
```

`spell_end_by_owner` 的结算调用改为 `self.settle_one_spell(slot, crate::spell::SPELL_END_MANUAL);`（删掉原先的 captured/reason 局部变量）。
`SPELL_FAIL_CAPTURE_LOST` / `SPELL_FAIL_TIMEOUT` 若因搬迁出现 unused import，按 clippy 提示收拾。
再跑 `grep -rn "settle_one_spell(" crates/stg-core/src`，把测试里的三参调用全部改成两参（`captured=true/false` 的旧测试按其场景换成 `SPELL_END_HP`/`SPELL_END_TIMEOUT`/`SPELL_END_MANUAL`，断言不变）。

`step.rs` `copy_into`，紧跟 `d.spell_seq = s.spell_seq;` 之后：

```rust
        d.spell_last_result = s.spell_last_result; // [u8; MAX_BOSSES]，结束方式读口（boss 换段刀）
```

`field.rs`（`FIELD_DAMAGE` 之后 + `fullscreen_clear_field` 之后）：

```rust
/// 标志位：清弹但**不转星星**（boss 换段刀 spec §6：超时结算 / `clear_bullets_at(..., stars=0)`）。
/// 弹照样标 `BULLET_CLEARED`、照样计入 `EVT_FIELD_CLEARED`。
pub const FIELD_NO_STAR: u8 = 1 << 2;
```

```rust
/// 全屏清弹、不给星——符卡超时结算专用（boss 换段刀 spec §3.2 ③）。几何与
/// [`fullscreen_clear_field`] 同源，只多一位 `FIELD_NO_STAR`。
pub(crate) fn fullscreen_clear_field_no_star() -> FieldInit {
    FieldInit {
        flags: FIELD_CLEAR_BULLETS | FIELD_NO_STAR,
        ..fullscreen_clear_field()
    }
}
```

`settle.rs` 趟一，把 `self.spawn_star_at(...)` 那行包成：

```rust
            // FIELD_NO_STAR 的区只清不转星（boss 换段刀 spec §6）；幂等门已保证一颗弹只处理一次，
            // 故多区重叠时由 hits 序中第一条命中的区决定，确定性。
            if self.fields.flags[h.active as usize] & crate::field::FIELD_NO_STAR == 0 {
                self.spawn_star_at(self.bullets.x[b], self.bullets.y[b], star_target);
            }
```

`events.rs`（`EVT_STAGE_CLEARED` 之后）：

```rust
/// 非符段结束（boss 换段刀 spec §3.2）：`a_index/a_gen` = 绑定 boss，`x`/`y` = boss 当帧位置
/// （句柄失效时为原点），`data = [spell_id, cause]`，`cause` 取 `SPELL_END_*`（1 血线 / 2 超时 / 3 手动）。
/// 普通符卡照旧发 `EVT_SPELL_CAPTURED`/`EVT_SPELL_FAILED`，不发本事件。
pub const EVT_PHASE_ENDED: u8 = 12;
```

`consts.rs` `structural` 段末尾（`REWIND_DEPTH` 之后）：

```rust
        //  符卡机构 flags 位与结束方式（boss 换段刀 2026-09-14）：`spell_begin` 第 6 参与
        //  `spell_result(slot)` 的返回值。此前脚本只能写字面量 1/2。
        SPELL_SURVIVAL:      u8 as int = crate::spell::SPELL_SURVIVAL;
        SPELL_NO_CLEAR:      u8 as int = crate::spell::SPELL_NO_CLEAR;
        SPELL_NONSPELL:      u8 as int = crate::spell::SPELL_NONSPELL;
        SPELL_END_HP:        u8 as int = crate::spell::SPELL_END_HP;
        SPELL_END_TIMEOUT:   u8 as int = crate::spell::SPELL_END_TIMEOUT;
        SPELL_END_MANUAL:    u8 as int = crate::spell::SPELL_END_MANUAL;
```

注入前先查撞名（脚本不得重声明引擎常量）：
Run: `grep -rn "const SPELL_SURVIVAL\|const SPELL_NO_CLEAR\|const SPELL_NONSPELL\|const SPELL_END_" --include=*.ecl --include=*.md --include=*.rs . | grep -v "crates/stg-core/src/spell.rs"`
Expected: 无输出（有则把那处 `const` 删掉改用注入常量）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core 2>&1 | tail -5`
Expected: 全绿（含既有符卡结束矩阵测试，断言值不变）。
Run: `cargo test -p stg-ecl-compiler 2>&1 | tail -3`（引擎常量注入进编译器，确认无撞名）
Expected: 全绿。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add -A crates/stg-core
git commit -m "feat(core): 符卡结算按结束方式分派——非符段 SPELL_NONSPELL、超时钉血、超时清弹不给星（FIELD_NO_STAR）、spell_last_result、EVT_PHASE_ENDED"
```

---

### Task 2: 敌判定写口、`$self_enemy`、`kill_all_enemies`（核 + syscall）

**Files:**
- Modify: `crates/stg-core/src/enemy.rs`（新位 + 清场模式常量）
- Modify: `crates/stg-core/src/world.rs`（5 个写 API）
- Modify: `crates/stg-core/src/world/collide.rs`（行 3 跳过 `ENEMY_NO_BODY` + 测试）
- Modify: `crates/stg-core/src/world/settle.rs`（无敌判别测试）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（6 号、`syscall_implemented`、`dispatch`、`frozen_table`、测试）
- Modify: `crates/stg-core/src/spell.rs`（清场杀绑定 boss 的测试）
- Modify: `crates/stg-core/src/consts.rs`（注入 4 个常量）

**Interfaces:**
- Consumes: Task 1 的 `SPELL_END_HP`、`spell_last_result`。
- Produces: `crate::enemy::{ENEMY_NO_BODY, ENEMY_KILLALL_EXEMPT, KILL_SILENT, KILL_DIE}: u8`；
  `WorldBody::{set_enemy_invuln(h, u16), set_enemy_hitbox(h, Fx), set_enemy_hurtbox(h, Fx), set_enemy_flags(h, u8, bool), kill_all_enemies(Option<EnemyHandle>, bool, &WorldTables)}`；
  `syscall::{SYS_SELF_ENEMY=26, SYS_SET_INVULN=440, SYS_SET_HITBOX=441, SYS_SET_HURTBOX=442, SYS_SET_ENEMY_FLAG=443, SYS_KILL_ALL_ENEMIES=531}`。

- [ ] **Step 1: 写失败测试**

collide.rs 测试模块：

```rust
    /// `ENEMY_NO_BODY`（boss 换段刀 spec §5.1 / §8-8）：圆心重合也不收行 3，但行 4 照收（仍吃弹）。
    #[test]
    fn collide_body_skips_no_body_enemy_but_shots_still_hit() {
        use crate::events::{ROW_BODY_PLAYER_HIT, ROW_SHOT_ENEMY};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        let e = spawn_enemy(&mut w, 0, 100, 5);
        let ei = w.body.enemies.get(e).unwrap();
        w.body.enemies.flags[ei] |= crate::enemy::ENEMY_NO_BODY;
        w.body.create_player_shot(crate::shots::ShotInit {
            x: Fx::ZERO,
            y: Fx::from_int(100),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 1,
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
        let count = |row| {
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == row)
                .count()
        };
        assert_eq!(count(ROW_BODY_PLAYER_HIT), 0, "不体碰");
        assert_eq!(count(ROW_SHOT_ENEMY), 1, "仍吃自机弹");
    }

    /// 判别式（D8 教训）：`set_enemy_hitbox` 只改行 3，`set_enemy_hurtbox` 只改行 7。
    /// 自机 (0,100) hit 2.5；敌 (30,100) 默认 radius 12 / hurtbox 16；伤害区 (-20,100) r=20，距敌 50。
    /// 默认：12+2.5<30 不体碰、20+16<50 不受击；各改成 40 后恰好翻转各自那一行。
    #[test]
    fn set_enemy_hitbox_moves_body_row_only_and_hurtbox_moves_field_row_only() {
        use crate::events::{ROW_BODY_PLAYER_HIT, ROW_FIELD_ENEMY};
        use crate::field::FIELD_DAMAGE;
        fn run(hit: Option<i32>, hurt: Option<i32>) -> (usize, usize) {
            let mut w = crate::step::World::new(1);
            w.body.players[0].x = Fx::ZERO;
            w.body.players[0].y = Fx::from_int(100);
            let e = spawn_enemy(&mut w, 30, 100, 5);
            spawn_field(&mut w, -20, 100, 20, FIELD_DAMAGE, 1);
            if let Some(r) = hit {
                w.body.set_enemy_hitbox(e, Fx::from_int(r));
            }
            if let Some(r) = hurt {
                w.body.set_enemy_hurtbox(e, Fx::from_int(r));
            }
            #[cfg(debug_assertions)]
            {
                w.body.phase_guard = PH_COLLIDE;
            }
            w.body.collide(&crate::tables::TABLES_V0);
            let count = |row| {
                (0..w.body.hits_len as usize)
                    .filter(|&k| w.body.hits[k].row == row)
                    .count()
            };
            (count(ROW_BODY_PLAYER_HIT), count(ROW_FIELD_ENEMY))
        }
        assert_eq!(run(None, None), (0, 0));
        assert_eq!(run(Some(40), None), (1, 0), "hitbox 只改体碰");
        assert_eq!(run(None, Some(40)), (0, 1), "hurtbox 只改受击");
    }
```

settle.rs 测试模块（无敌此前只有字段与判定、没有判别测试）：

```rust
    /// `set_enemy_invuln`（boss 换段刀 spec §5.1）：无敌期间自机弹重叠既不掉血也不发命中事件；
    /// 解除后同一发照打（对照腿，证明是 invuln 挡的而不是几何没碰上）。
    #[test]
    fn invulnerable_enemy_takes_no_damage_and_no_hit_event() {
        use crate::events::EVT_SHOT_HIT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 100, 50);
        w.body.set_enemy_invuln(e, 5);
        w.body.create_player_shot(crate::shots::ShotInit {
            x: Fx::ZERO,
            y: Fx::from_int(100),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 7,
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
        let i = w.body.enemies.get(e).unwrap();
        assert_eq!(w.body.enemies.hp[i], 50);
        assert!(
            (0..w.body.frame_events_len as usize)
                .all(|k| w.body.frame_events[k].kind != EVT_SHOT_HIT_ENEMY)
        );

        w.body.set_enemy_invuln(e, 0);
        w.body.hits_len = 0;
        w.body.frame_events_len = 0;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(w.body.enemies.hp[i], 43, "解除后同一发打得进");
    }
```

syscall.rs 测试模块：

```rust
    /// 440 `set_invuln`：写入；`-1`/`65536` 越界整条 no-op + 违约；非敌 owner Fault。
    #[test]
    fn set_invuln_writes_rejects_out_of_range_and_faults_for_non_enemy() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        let i = eh.index as usize;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_INVULN, &[120]).is_ok());
        assert_eq!(w.body.enemies.invuln[i], 120);
        for bad in [-1, 65536] {
            let v0 = w.body.diag.contract_viol;
            assert!(call(&mut w, &ecl, &mut task, SYS_SET_INVULN, &[bad]).is_ok());
            assert_eq!(w.body.enemies.invuln[i], 120, "越界 {bad} 整条 no-op");
            assert_eq!(w.body.diag.contract_viol, v0 + 1);
        }
        let mut stage = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut stage, SYS_SET_INVULN, &[1]),
            Err(FAULT_BAD_OP)
        );
    }

    /// 441/442：各写各的半径，互不串位；负半径钳 0 并计违约。
    #[test]
    fn set_hitbox_and_hurtbox_write_their_own_radius_and_clamp() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        let i = eh.index as usize;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_HITBOX, &[Fx::from_int(40).raw()]).is_ok());
        assert_eq!(w.body.enemies.radius[i], Fx::from_int(40));
        assert_eq!(w.body.enemies.hurtbox[i], Fx::from_int(16));
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_HURTBOX, &[Fx::from_int(24).raw()]).is_ok());
        assert_eq!(w.body.enemies.hurtbox[i], Fx::from_int(24));
        assert_eq!(w.body.enemies.radius[i], Fx::from_int(40));
        let v0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_HITBOX, &[Fx::from_int(-5).raw()]).is_ok());
        assert_eq!(w.body.enemies.radius[i], Fx::ZERO);
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
    }

    /// 443 `set_enemy_flag`：只收 NO_BODY|KILLALL_EXEMPT 的非空子集；含 DYING/未知位/0 整条拒。
    #[test]
    fn set_enemy_flag_accepts_only_settable_bits() {
        use crate::enemy::{ENEMY_DYING, ENEMY_KILLALL_EXEMPT, ENEMY_NO_BODY};
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        let i = eh.index as usize;
        let (nb, ex) = (ENEMY_NO_BODY as i32, ENEMY_KILLALL_EXEMPT as i32);
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[nb | ex, 1]).is_ok());
        assert_eq!(w.body.enemies.flags[i], ENEMY_NO_BODY | ENEMY_KILLALL_EXEMPT);
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[ex, 0]).is_ok());
        assert_eq!(w.body.enemies.flags[i], ENEMY_NO_BODY);
        for bad in [0, ENEMY_DYING as i32, 8, nb | ENEMY_DYING as i32] {
            let v0 = w.body.diag.contract_viol;
            assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[bad, 1]).is_ok());
            assert_eq!(w.body.enemies.flags[i], ENEMY_NO_BODY, "坏掩码 {bad} 不改 flags");
            assert_eq!(w.body.diag.contract_viol, v0 + 1);
        }
    }

    /// 026 `$self_enemy`：敌 owner 押打包敌号；非敌押 -1（打包值 0 合法，故不读 0）。
    #[test]
    fn self_enemy_pushes_packed_handle_or_minus_one() {
        let (mut w, ecl) = fresh();
        let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
        assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ENEMY, &[]).is_ok());
        assert_eq!(task.stack[0], pack_enemy_handle(eh));
        let mut stage = Task::default();
        assert!(call(&mut w, &ecl, &mut stage, SYS_SELF_ENEMY, &[]).is_ok());
        assert_eq!(stage.stack[0], -1);
    }

    /// 531 静默模式：跳过调用者 / 免清 / 已死；被杀者无事件、无加分、无掉落。
    #[test]
    fn kill_all_enemies_silent_skips_caller_exempt_and_dying() {
        use crate::enemy::{ENEMY_DYING, ENEMY_KILLALL_EXEMPT, KILL_SILENT};
        let (mut w, ecl) = fresh();
        let (caller, mut task) = enemy_owner_task(&mut w, 10, 0);
        let exempt = crate::world::test_support::spawn_enemy(&mut w, 10, 0, 10);
        let dying = crate::world::test_support::spawn_enemy(&mut w, 20, 0, 10);
        let normal = crate::world::test_support::spawn_enemy(&mut w, 30, 0, 10);
        w.body.enemies.flags[exempt.index as usize] |= ENEMY_KILLALL_EXEMPT;
        w.body.enemies.flags[dying.index as usize] |= ENEMY_DYING;
        load_drop_table_1(&mut w, normal);
        let score0 = w.body.players[0].score;
        let ev0 = w.body.frame_events_len;
        assert!(call(&mut w, &ecl, &mut task, SYS_KILL_ALL_ENEMIES, &[KILL_SILENT as i32]).is_ok());
        let is_dying = |w: &World, h: EnemyHandle| w.body.enemies.flags[h.index as usize] & ENEMY_DYING != 0;
        assert!(!is_dying(&w, caller), "调用者自己不杀");
        assert!(!is_dying(&w, exempt), "免清位不杀");
        assert!(is_dying(&w, normal));
        assert_eq!(w.body.players[0].score, score0, "静默不加分");
        assert_eq!(w.body.frame_events_len, ev0, "静默不发事件");
        assert_eq!(w.body.items.iter_alive().count(), 0, "静默不掉落");
    }

    /// 531 击破模式走 die() 全套；坏 mode 整条 no-op + 违约。STAGE owner 无调用者豁免。
    #[test]
    fn kill_all_enemies_die_mode_runs_full_death_and_bad_mode_is_noop() {
        use crate::enemy::{ENEMY_DYING, KILL_DIE};
        use crate::events::EVT_ENEMY_DIED;
        let (mut w, ecl) = fresh();
        let e = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 10);
        load_drop_table_1(&mut w, e);
        let mut stage = Task::default();
        let v0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut stage, SYS_KILL_ALL_ENEMIES, &[2]).is_ok());
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
        assert_eq!(w.body.enemies.flags[e.index as usize] & ENEMY_DYING, 0, "坏 mode 不杀");

        let score0 = w.body.players[0].score;
        assert!(call(&mut w, &ecl, &mut stage, SYS_KILL_ALL_ENEMIES, &[KILL_DIE as i32]).is_ok());
        assert_ne!(w.body.enemies.flags[e.index as usize] & ENEMY_DYING, 0);
        assert_eq!(w.body.players[0].score, score0 + 100, "test_support 敌 score=100");
        assert_eq!(
            w.body.frame_events[w.body.frame_events_len as usize - 1].kind,
            EVT_ENEMY_DIED
        );
        assert_eq!(w.body.items.iter_alive().count(), 3, "掉落表 1 = 3 颗");
    }
```

frozen_table 加 6 行（放在各族对应位置）：

```rust
            (SYS_SELF_ENEMY, "self_enemy", 0),
            (SYS_SET_INVULN, "set_invuln", 4),
            (SYS_SET_HITBOX, "set_hitbox", 4),
            (SYS_SET_HURTBOX, "set_hurtbox", 4),
            (SYS_SET_ENEMY_FLAG, "set_enemy_flag", 4),
            (SYS_KILL_ALL_ENEMIES, "kill_all_enemies", 5),
```

spell.rs 测试模块：

```rust
    /// 清场杀掉绑定 boss → 当帧符卡趟按 HP 路径结算（spec §5.3）。
    #[test]
    fn kill_all_enemies_on_bound_boss_settles_as_hp_end() {
        let (mut w, boss) = world_with_boss(1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 60, 0, 0, 0));
        w.body
            .kill_all_enemies(None, false, &crate::tables::TABLES_V0);
        w.body.settle_spells(&crate::tables::TABLES_V0);
        assert_eq!(w.body.spell_last_result[0], SPELL_END_HP);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core set_invuln set_hitbox set_enemy_flag self_enemy kill_all collide_body_skips_no_body invulnerable_enemy 2>&1 | tail -10`
Expected: 编译失败（新常量/API/号未定义）。

- [ ] **Step 3: 实现**

`enemy.rs`（`ENEMY_DYING` 之后）：

```rust
/// `flags` 位：不与自机体碰（碰撞行 3 跳过；仍吃自机弹）。脚本经 `set_enemy_flag` 写（boss 换段刀）。
pub const ENEMY_NO_BODY: u8 = 1 << 1;
/// `flags` 位：`kill_all_enemies` 不杀它（boss 换段刀）。
pub const ENEMY_KILLALL_EXEMPT: u8 = 1 << 2;
/// `kill_all_enemies` 模式：静默退场（同 D9 主任务 return：不掉落不加分不发事件）。
pub const KILL_SILENT: u8 = 0;
/// `kill_all_enemies` 模式：逐只走 `kill_enemy`（同 `die()`）。
pub const KILL_DIE: u8 = 1;
```

`world.rs`（`set_anm_state` 之后）：

```rust
    /// 敌无敌帧（boss 换段刀 spec §5.1）：覆写 `invuln`，0 = 取消。P4-b：悬垂句柄 → no-op + STALE。
    pub fn set_enemy_invuln(&mut self, h: EnemyHandle, frames: u16) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        self.enemies.invuln[i] = frames;
        self.last_status = STATUS_OK;
    }

    /// 敌体碰半径（碰撞行 3）。钳制口径同 `create_enemy`（`[0, MAX_ENTITY_RADIUS]`，钳了计违约）。
    pub fn set_enemy_hitbox(&mut self, h: EnemyHandle, mut r: Fx) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        if Self::clamp_radius(&mut r) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        self.enemies.radius[i] = r;
        self.last_status = STATUS_OK;
    }

    /// 敌受击半径（碰撞行 4/7）。口径同 [`Self::set_enemy_hitbox`]。
    pub fn set_enemy_hurtbox(&mut self, h: EnemyHandle, mut r: Fx) {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        if Self::clamp_radius(&mut r) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        self.enemies.hurtbox[i] = r;
        self.last_status = STATUS_OK;
    }

    /// 置/清敌标志位。`mask` 的合法性（只许 NO_BODY|KILLALL_EXEMPT）由 syscall 层验，
    /// 本 API 信任调用方（不许碰 `ENEMY_DYING`——那是 settle/cleanup 的状态机）。
    pub fn set_enemy_flags(&mut self, h: EnemyHandle, mask: u8, on: bool) {
        debug_assert_eq!(mask & crate::enemy::ENEMY_DYING, 0, "脚本写口不许动 dying 位");
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return;
        };
        if on {
            self.enemies.flags[i] |= mask;
        } else {
            self.enemies.flags[i] &= !mask;
        }
        self.last_status = STATUS_OK;
    }

    /// 清场（boss 换段刀 spec §5.3）：池索引升序（I4），跳过 `except`、已 dying、带免清位的敌。
    /// `die=false` 静默（只置 dying，同 D9 退场）；`die=true` 逐只 `kill_enemy`（同 `die()`）。
    pub fn kill_all_enemies(
        &mut self,
        except: Option<EnemyHandle>,
        die: bool,
        tables: &crate::tables::WorldTables,
    ) {
        let skip = crate::enemy::ENEMY_DYING | crate::enemy::ENEMY_KILLALL_EXEMPT;
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let e = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if except.is_some_and(|h| {
                    h.index as usize == e && h.generation == self.enemies.generation[e]
                }) {
                    continue;
                }
                if self.enemies.flags[e] & skip != 0 {
                    continue;
                }
                if die {
                    self.kill_enemy(e, tables);
                } else {
                    self.enemies.flags[e] |= crate::enemy::ENEMY_DYING;
                }
            }
        }
        self.last_status = STATUS_OK;
    }
```

`collide.rs` `collide_body_player` 内层循环，`let dx = ...` 之前：

```rust
                    if self.enemies.flags[e] & crate::enemy::ENEMY_NO_BODY != 0 {
                        continue; // 脚本关了体碰（boss 换段刀 spec §5.1）；仍不查 dying（D-9）
                    }
```

并把模块头注释行 3 那句补上「（`ENEMY_NO_BODY` 跳过）」。

`syscall.rs` 常量（各放进所属族的常量区，照邻居写 doc 注释，引 spec 节号）：

```rust
pub const SYS_SELF_ENEMY: u16 = 26;
pub const SYS_SET_INVULN: u16 = 440;
pub const SYS_SET_HITBOX: u16 = 441;
pub const SYS_SET_HURTBOX: u16 = 442;
pub const SYS_SET_ENEMY_FLAG: u16 = 443;
pub const SYS_KILL_ALL_ENEMIES: u16 = 531;
```

`syscall_implemented` 的 `matches!` 各族追加这 6 个名字。`dispatch`：

```rust
        SYS_SELF_ENEMY => {
            // 非敌押 -1 而非 0：打包敌号 0 合法（spec §5.2），与族内其它变量「非敌读 0」有意不同。
            let v = if task.owner_kind == OWNER_ENEMY {
                pack_enemy_handle(EnemyHandle {
                    index: task.owner_index,
                    generation: task.owner_gen,
                })
            } else {
                -1
            };
            push(task, v)
        }
```

```rust
        SYS_SET_INVULN => {
            let h = self_enemy_handle(task)?;
            let n = pop(task)?;
            let Ok(frames) = u16::try_from(n) else {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                return Ok(());
            };
            ctx.body.set_enemy_invuln(h, frames);
            Ok(())
        }
        SYS_SET_HITBOX => {
            let h = self_enemy_handle(task)?;
            let r = pop(task)?;
            ctx.body.set_enemy_hitbox(h, Fx::from_raw(r));
            Ok(())
        }
        SYS_SET_HURTBOX => {
            let h = self_enemy_handle(task)?;
            let r = pop(task)?;
            ctx.body.set_enemy_hurtbox(h, Fx::from_raw(r));
            Ok(())
        }
        SYS_SET_ENEMY_FLAG => {
            let h = self_enemy_handle(task)?;
            let on = pop(task)?;
            let mask = pop(task)?;
            const SETTABLE: i32 =
                (crate::enemy::ENEMY_NO_BODY | crate::enemy::ENEMY_KILLALL_EXEMPT) as i32;
            if mask == 0 || mask & !SETTABLE != 0 {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                return Ok(());
            }
            ctx.body.set_enemy_flags(h, mask as u8, on != 0);
            Ok(())
        }
```

```rust
        SYS_KILL_ALL_ENEMIES => {
            let mode = pop(task)?;
            let die = match mode {
                m if m == crate::enemy::KILL_SILENT as i32 => false,
                m if m == crate::enemy::KILL_DIE as i32 => true,
                _ => {
                    ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                    ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                    return Ok(());
                }
            };
            let except = (task.owner_kind == OWNER_ENEMY).then_some(EnemyHandle {
                index: task.owner_index,
                generation: task.owner_gen,
            });
            ctx.body.kill_all_enemies(except, die, ctx.tables);
            Ok(())
        }
```

`consts.rs` 追加：

```rust
        //  敌判定标志位与清场模式（boss 换段刀 2026-09-14）：`set_enemy_flag` / `kill_all_enemies` 参数。
        ENEMY_NO_BODY:        u8 as int = crate::enemy::ENEMY_NO_BODY;
        ENEMY_KILLALL_EXEMPT: u8 as int = crate::enemy::ENEMY_KILLALL_EXEMPT;
        KILL_SILENT:          u8 as int = crate::enemy::KILL_SILENT;
        KILL_DIE:             u8 as int = crate::enemy::KILL_DIE;
```

撞名检查：`grep -rn "const ENEMY_NO_BODY\|const ENEMY_KILLALL_EXEMPT\|const KILL_SILENT\|const KILL_DIE" --include=*.ecl --include=*.md .`，Expected 无输出。

`crates/stg-ecl-compiler/src/lang/mod.rs::opcodes_of` 文档里「`0xx` 族（`$` 引擎变量，000–032）……12 条里每一条都撞着一个 op」一句改为「000–032 共 13 条，其中 026 `self_enemy` 不是 op 号，其余 12 条各撞一个 op」。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core 2>&1 | tail -5`
Expected: 全绿（`frozen_table` 两条结构/白名单测试一并绿）。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add -A crates/stg-core crates/stg-ecl-compiler/src/lang/mod.rs
git commit -m "feat(core): 敌判定写口 440-443（set_invuln/hitbox/hurtbox/enemy_flag）、\$self_enemy(026)、kill_all_enemies(531)、碰撞行 3 跳过 ENEMY_NO_BODY"
```

---

### Task 3: `spell_result`（131）与 `clear_bullets_at`（541）

**Files:**
- Modify: `crates/stg-core/src/field.rs`（`clear_field_at` 构造口）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（2 号、`syscall_implemented`、`dispatch`、`frozen_table`、测试）
- Modify: `crates/stg-core/src/world/settle.rs`（半径几何测试）

**Interfaces:**
- Consumes: Task 1 的 `spell_last_result`、`FIELD_NO_STAR`。
- Produces: `syscall::{SYS_SPELL_RESULT = 131, SYS_CLEAR_BULLETS_AT = 541}`；`crate::field::clear_field_at(x: Fx, y: Fx, r: Fx, stars: bool) -> FieldInit`。

- [ ] **Step 1: 写失败测试**

syscall.rs 测试模块：

```rust
    /// 131 `spell_result`：读槽值；越界 / 负槽号押 0 + 违约。owner 无限制（STAGE 可读）。
    #[test]
    fn spell_result_reads_slot_and_rejects_out_of_range() {
        let (mut w, ecl) = fresh();
        w.body.spell_last_result[1] = crate::spell::SPELL_END_TIMEOUT;
        let mut stage = Task::default();
        assert!(call(&mut w, &ecl, &mut stage, SYS_SPELL_RESULT, &[1]).is_ok());
        assert_eq!(stage.stack[0], crate::spell::SPELL_END_TIMEOUT as i32);
        for bad in [-1, crate::boss::MAX_BOSSES as i32] {
            let mut t = Task::default();
            let v0 = w.body.diag.contract_viol;
            assert!(call(&mut w, &ecl, &mut t, SYS_SPELL_RESULT, &[bad]).is_ok());
            assert_eq!(t.stack[0], 0);
            assert_eq!(w.body.diag.contract_viol, v0 + 1);
        }
    }

    /// 541 `clear_bullets_at`：按参数铺一帧清弹区；stars=0 带 FIELD_NO_STAR。
    #[test]
    fn clear_bullets_at_creates_one_frame_field_with_star_bit() {
        use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
        let (mut w, ecl) = fresh();
        let mut stage = Task::default();
        let args = [
            Fx::from_int(-30).raw(),
            Fx::from_int(200).raw(),
            Fx::from_int(40).raw(),
            0,
        ];
        assert!(call(&mut w, &ecl, &mut stage, SYS_CLEAR_BULLETS_AT, &args).is_ok());
        let f = w.body.fields.iter_alive().next().expect("应建清弹区");
        assert_eq!(w.body.fields.x[f], Fx::from_int(-30));
        assert_eq!(w.body.fields.y[f], Fx::from_int(200));
        assert_eq!(w.body.fields.radius[f], Fx::from_int(40));
        assert_eq!(w.body.fields.life[f], 1);
        assert_eq!(w.body.fields.dmg_per_frame[f], 0);
        assert_eq!(w.body.fields.flags[f], FIELD_CLEAR_BULLETS | FIELD_NO_STAR);

        let (mut w2, _) = fresh();
        let mut args2 = args;
        args2[3] = 1;
        assert!(call(&mut w2, &ecl, &mut stage, SYS_CLEAR_BULLETS_AT, &args2).is_ok());
        let f2 = w2.body.fields.iter_alive().next().unwrap();
        assert_eq!(w2.body.fields.flags[f2], FIELD_CLEAR_BULLETS);
    }
```

frozen_table 加：

```rust
            (SYS_SPELL_RESULT, "spell_result", 1),
            (SYS_CLEAR_BULLETS_AT, "clear_bullets_at", 5),
```

settle.rs 测试模块：

```rust
    /// `clear_field_at` 几何判别：圆内弹被清、圆外弹保留（证明半径真的生效、不是全屏）。
    #[test]
    fn clear_field_at_clears_inside_and_keeps_outside() {
        let mut w = crate::step::World::new(1);
        w.body.create_field(crate::field::clear_field_at(
            Fx::ZERO,
            Fx::from_int(100),
            Fx::from_int(40),
            true,
        ));
        let inside = bullet_at(&mut w, 10, 100);
        let outside = bullet_at(&mut w, 150, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        w.body.settle(&crate::tables::TABLES_V0);
        let cleared = |w: &crate::step::World, h| {
            let i = w.body.bullets.get(h).unwrap();
            w.body.bullets.flags[i] & crate::bullets::BULLET_CLEARED != 0
        };
        assert!(cleared(&w, inside));
        assert!(!cleared(&w, outside));
        assert_eq!(w.body.items.iter_alive().count(), 1, "stars=true 给星");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core spell_result_reads clear_bullets_at clear_field_at 2>&1 | tail -8`
Expected: 编译失败。

- [ ] **Step 3: 实现**

`field.rs`（`fullscreen_clear_field_no_star` 之后）：

```rust
/// 圆形一帧清弹区——`clear_bullets_at` syscall（541）的构造口（boss 换段刀 spec §6）。
/// 半径钳制交给 `create_field`（P4-b）。`stars=false` 带 `FIELD_NO_STAR`。
pub(crate) fn clear_field_at(x: Fx, y: Fx, r: Fx, stars: bool) -> FieldInit {
    FieldInit {
        x,
        y,
        radius: r,
        dmg_per_frame: 0,
        life: 1,
        owner: 0,
        flags: if stars {
            FIELD_CLEAR_BULLETS
        } else {
            FIELD_CLEAR_BULLETS | FIELD_NO_STAR
        },
    }
}
```

`syscall.rs`：

```rust
pub const SYS_SPELL_RESULT: u16 = 131;
pub const SYS_CLEAR_BULLETS_AT: u16 = 541;
```

`syscall_implemented` 追加两名；`dispatch`：

```rust
        SYS_SPELL_RESULT => {
            let slot = pop(task)?;
            let v = match usize::try_from(slot) {
                Ok(s) if s < crate::boss::MAX_BOSSES => ctx.body.spell_last_result[s] as i32,
                _ => {
                    ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                    ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
                    0
                }
            };
            push(task, v)
        }
```

```rust
        SYS_CLEAR_BULLETS_AT => {
            let stars = pop(task)?;
            let r = pop(task)?;
            let y = pop(task)?;
            let x = pop(task)?;
            ctx.body.create_field(crate::field::clear_field_at(
                Fx::from_raw(x),
                Fx::from_raw(y),
                Fx::from_raw(r),
                stars != 0,
            ));
            Ok(())
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core 2>&1 | tail -5`
Expected: 全绿。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add -A crates/stg-core
git commit -m "feat(core): spell_result(131) 结束方式读口、clear_bullets_at(541) 半径清弹 + 不给星"
```

---

### Task 4: `spawn_enemy` 带参全链路（syscall 210 调用约定 + 编译器）

**Files:**
- Modify: `crates/stg-core/src/ecl/task.rs`（`write_args` 助手）
- Modify: `crates/stg-core/src/ecl/vm.rs`（`OP_SPAWN` 改调助手）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（`sys_spawn_enemy` 新弹栈序 + 既有 10 处测试 args 末尾追加 `0` + 新测试）
- Modify: `crates/stg-ecl-compiler/src/lib.rs`（builder `sys_spawn_enemy` 追压 argc 0）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/typed_ast.rs`（`CallArg::SubRefArgs`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/exprs.rs`（task 位接受 `sub(实参…)`）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`（降低 + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/slots.rs`（栈深）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/tests.rs`（报错测试）
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（端到端测试）

**Interfaces:**
- Produces: `TaskPool::write_args(&mut self, idx: u16, args: &[i32])`；
  syscall 210 压栈序 `x, y, hp, drop_table, score, sprite, task_sub, arg0..arg(n-1), argc`；
  `CallArg::SubRefArgs(String, Vec<TypedExpr>)`（仅 `spawn_enemy` 的 task 位产生）。

- [ ] **Step 1: 写失败测试（核侧）**

先把 syscall.rs 里既有 10 处 `call(..., SYS_SPAWN_ENEMY, &args)` 的 `args` 数组末尾各追加一个 `0`（argc），例如 `spawn_enemy_with_task_binds_owner_and_main_task` 里：

```rust
        let args = [
            Fx::from_int(0).raw(),
            Fx::from_int(80).raw(),
            10,
            0,
            0,
            5,
            1,
            0, // argc（boss 换段刀：210 调用约定追加）
        ];
```

（`grep -n "SYS_SPAWN_ENEMY, &args" crates/stg-core/src/ecl/syscall.rs` 逐处改；`args` 若是 `[i32; 7]` 类型标注一并改 8。）

新测试：

```rust
    /// 1 参 Async sub（raw=1）的镜像——spawn_enemy 带参测试专用。
    fn one_arg_async_image() -> EclImage {
        test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![EclValueType::Int]),
            ],
            vec![EntryInit::new("zako", 1)],
            Some(0),
        )
    }

    /// 带参生成（boss 换段刀 spec §4.2）：同一帧两只敌各拿自己的实参——globals 顶替做不到的判别腿。
    #[test]
    fn spawn_enemy_with_args_gives_each_task_its_own_args_same_frame() {
        let ecl = one_arg_async_image();
        let mut w = World::new(1);
        let mut task = Task::default();
        for v in [111, 222] {
            let args = [0, Fx::from_int(80).raw(), 10, 0, 0, 0, 1, v, 1];
            assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        }
        let got: Vec<i32> = w
            .tasks
            .iter_alive()
            .map(|i| w.tasks.slots[i].locals[0])
            .collect();
        assert_eq!(got, vec![111, 222]);
    }

    /// 带参门禁：个数不符 / none 带参 → Fault(0) 且敌未建；argc 超 LOCALS 或栈不够 → Fault(2)。
    #[test]
    fn spawn_enemy_with_args_rejects_bad_arity_before_creating_enemy() {
        let ecl = one_arg_async_image();
        let cases: [(&[i32], u8); 4] = [
            (&[0, 0, 10, 0, 0, 0, 1, 0], FAULT_BAD_OP),       // 1 参 sub 给 0 参
            (&[0, 0, 10, 0, 0, 0, -1, 7, 1], FAULT_BAD_OP),   // none 带参
            (&[0, 0, 10, 0, 0, 0, 1, 65], FAULT_STACK),       // argc > LOCALS
            (&[0, 0, 10, 0, 0, 0, 1, 3], FAULT_STACK),        // 栈里不够 argc+7
        ];
        for (args, fault) in cases {
            let mut w = World::new(1);
            let mut task = Task::default();
            assert_eq!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, args), Err(fault), "{args:?}");
            assert_eq!(w.body.enemies.iter_alive().count(), 0, "{args:?}：敌不得建出");
        }
    }
```

（`EclValueType` 若测试模块未导入，加 `use crate::ecl::image::EclValueType;`。）

- [ ] **Step 2: 跑核侧测试确认失败**

Run: `cargo test -p stg-core spawn_enemy 2>&1 | tail -10`
Expected: 新两条失败（旧实现把 argc 当 task_script 弹）；既有 spawn_enemy 测试也红（多压了一个值）——Step 3 后应全部转绿。

- [ ] **Step 3: 核侧实现**

`task.rs`（`spawn` 之后）：

```rust
    /// spawn 之后把实参写进新任务 `locals[0..args.len())`（`OP_SPAWN` 与 `spawn_enemy` 共用，
    /// boss 换段刀）。新槽 locals 已由 [`Self::spawn`] 全零初始化，`args` 为空时 no-op。
    pub(crate) fn write_args(&mut self, idx: u16, args: &[i32]) {
        self.slots[idx as usize].locals[..args.len()].copy_from_slice(args);
    }
```

`vm.rs` `OP_SPAWN` 的 `Some(idx)` 臂：把 `ctx.tasks.slots[idx as usize].locals[..argc].copy_from_slice(&args[..argc]);` 换成 `ctx.tasks.write_args(idx, &args[..argc]);`（上方注释保留）。

`syscall.rs` `sys_spawn_enemy` 开头改为（其余建敌代码不变）：

```rust
fn sys_spawn_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    // boss 换段刀 spec §4.2：压栈序 `x,y,hp,drop,score,sprite,task_sub, arg0..arg(n-1), argc`。
    // 门禁全部先于建敌（零副作用）。argc/栈深口径同 OP_SPAWN。
    let argc = usize::try_from(pop(task)?)
        .ok()
        .filter(|&n| n <= LOCALS)
        .ok_or(FAULT_STACK)?;
    if (task.sp as usize) < argc + 7 {
        return Err(FAULT_STACK);
    }
    let mut args = [0i32; LOCALS];
    for k in (0..argc).rev() {
        args[k] = pop(task)?;
    }
    let task_script = pop(task)?;
    let sprite = pop(task)?;
    let score = pop(task)?;
    let drop_table = pop(task)?;
    let hp = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;

    // task 号先验后建：在册 + Async + 形参个数 == argc；none 不许带参。
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async
            || ctx.ecl.param_types(sub).is_none_or(|p| p.len() != argc)
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else if argc > 0 {
        return Err(FAULT_BAD_OP);
    } else {
        None
    };
```

任务 spawn 成功臂追加写参：

```rust
            Some(slot) => {
                ctx.tasks.write_args(slot, &args[..argc]);
                ctx.body.enemies.main_task[handle.index as usize] = slot as u32 + 1;
            }
```

同步更新 syscall.rs 顶部 `SYS_SPAWN_ENEMY` 常量的 doc 注释：写明新压栈序与「无参时 argc=0」。

Run: `cargo test -p stg-core 2>&1 | tail -5`
Expected: 全绿。

- [ ] **Step 4: 编译器侧失败测试**

`typeck/tests.rs` 追加：

```rust
#[test]
fn spawn_enemy_task_accepts_sub_call_with_matching_args() {
    ok("async sub zako(dir: int, v: fx) { loop { wait(1); } }\n\
        sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, zako(1, 2.5fx)); }");
}

#[test]
fn spawn_enemy_task_args_are_type_and_arity_checked() {
    let src_arity = "async sub zako(dir: int) { loop { wait(1); } }\n\
                     sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, zako(1, 2)); }";
    assert!(err(src_arity).iter().any(|e| e.msg.contains("参数个数")));
    let src_ty = "async sub zako(dir: int) { loop { wait(1); } }\n\
                  sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, zako(1.0fx)); }";
    assert!(!err(src_ty).is_empty());
    let src_none = "sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, none(1)); }";
    assert!(!err(src_none).is_empty());
    let src_sync = "sub zako(dir: int) { }\n\
                    sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, zako(1)); }";
    assert!(err(src_sync).iter().any(|e| e.msg.contains("async sub")));
}

#[test]
fn fire_task_rejects_sub_call_with_args_pointing_to_spawn_enemy() {
    let src = "async sub t(a: int) { }\n\
               sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, none, t(1)); }";
    assert!(
        err(src)
            .iter()
            .any(|e| e.msg.contains("只有 spawn_enemy")),
        "{:?}",
        err(src)
    );
}
```

`lang/mod.rs` 端到端测试（放在既有 `start_main` e2e 测试旁）：

```rust
        /// 带参 spawn_enemy 端到端（boss 换段刀 spec §4）：同一帧生成两只，各自把实参写进
        /// 不同 globals 槽；次帧首跑后两槽分别是各自的实参。
        #[test]
        fn spawn_enemy_with_args_end_to_end_same_frame() {
            let src = "async sub zako(slot: int, v: int) {\n\
                         set_global(slot, v);\n\
                         loop { wait(1); }\n\
                       }\n\
                       sub main() {\n\
                         _ = spawn_enemy(-50.0fx, 50.0fx, 10, 0, 0, 0, zako(20, 111));\n\
                         _ = spawn_enemy(50.0fx, 50.0fx, 10, 0, 0, 0, zako(21, 222));\n\
                         loop { wait(1); }\n\
                       }";
            let img = compile(src, "t.ecl").expect("应编译成功");
            let t = stg_core::tables::TABLES_V0;
            let mut w = stg_core::step::World::new(1);
            w.start_main(&img).expect("main 应能派生");
            for f in 0..3 {
                stg_core::step::step(&mut w, &t, &img, &stg_core::input::InputFrame::empty(f));
            }
            let g = w.body.view().globals();
            assert_eq!((g[20], g[21]), (111, 222));
            assert_eq!(w.body.view().diag().task_faults, 0);
        }
```

（若 `TABLES_V0` 不是 `Copy`，改为 `let t = &stg_core::tables::TABLES_V0;` 并把 `&t` 换成 `t`。）

`codegen.rs` 测试：既有 `spawn_enemy_task_param_lowers_like_fire` 保留；无需新增字节码断言（e2e 已覆盖）。

Run: `cargo test -p stg-ecl-compiler spawn_enemy fire_task_rejects 2>&1 | tail -10`
Expected: 新测试失败（`zako(1, 2.5fx)` 被当成求值表达式拒掉）；**且**既有 e2e 若跑到 spawn_enemy 会因缺 argc 而 Fault——Step 5 修。

- [ ] **Step 5: 编译器侧实现**

`typed_ast.rs` `CallArg`：

```rust
    /// `name(实参…)`：带参 sub 引用——**只由 `spawn_enemy` 的 task 位产生**（boss 换段刀 spec §4.1）。
    /// 实参已按目标 async sub 签名判型；codegen 降低为「task 号, 实参…, argc」。
    SubRefArgs(String, Vec<TypedExpr>),
```

`const_val` 的 `None` 臂加 `| CallArg::SubRefArgs(..)`。

`exprs.rs` `check_builtin_call_args` 的 `ParamKind::SubRef` 臂替换为：

```rust
                ParamKind::SubRef => match a {
                    Expr::Call { name, args: sub_args, span: cspan } => {
                        match self.resolve_sub_ref_with_args(b.name, name, sub_args, *cspan, locals) {
                            Some(arg) => out.push(arg),
                            None => ok = false,
                        }
                    }
                    _ => match self.resolve_ident_ref(a, span, RefKind::Sub, b.name) {
                        Some(r) => out.push(CallArg::SubRef(r)),
                        None => ok = false,
                    },
                },
```

并在 `impl` 内新增：

```rust
    /// task 位的 `name(实参…)` 写法（boss 换段刀 spec §4.1）：只 `spawn_enemy` 接受；
    /// 目标须为已声明 async sub，实参按其签名判型（同 `spawn f(args);` 的规则，复用
    /// `check_sub_call_args`——那里的同步调用边记录对实参内嵌调用照常生效）。
    fn resolve_sub_ref_with_args(
        &mut self,
        builtin_name: &str,
        name: &str,
        args: &[Expr],
        span: Span,
        locals: &LocalScope,
    ) -> Option<CallArg> {
        if builtin_name != "spawn_enemy" {
            self.err(
                span,
                format!(
                    "'{builtin_name}' 的 task 引用不能带实参（目前只有 spawn_enemy 的 task 位支持带参）"
                ),
            );
            return None;
        }
        if name == "none" {
            self.err(span, "'none' 不能带实参".into());
            return None;
        }
        let Some(sub) = self.subs.get(name).cloned() else {
            self.err(span, format!("未知的 sub 名 '{name}'"));
            return None;
        };
        if !sub.is_async {
            self.err(
                span,
                format!("'{name}' 用作 spawn_enemy 的 task 引用必须声明为 async sub"),
            );
            return None;
        }
        let typed = self.check_sub_call_args(&sub.params, args, span, locals)?;
        let exprs = typed
            .into_iter()
            .map(|a| match a {
                CallArg::Val(t) => t,
                _ => unreachable!("check_sub_call_args 只产 Val"),
            })
            .collect();
        Some(CallArg::SubRefArgs(name.to_string(), exprs))
    }
```

（`self.subs` 的值类型若不是 `Clone`，改为先取 `params.clone()` 与 `is_async` 两个字段；`sub.params` 的类型须与 `check_sub_call_args` 的 `&[(String, Ty)]` 对齐——不一致时按 `exprs.rs:198` 附近 sub 调用处的现成转换写。）

同时把既有「task 引用必须是无参 async sub（派生不带实参——需要传参请用 spawn）」文案末尾改为「……需要传参请用 spawn；spawn_enemy 的 task 位可写 name(实参…)」，并同步 `typeck/tests.rs:826` 那条断言若依赖原文（它断言 `contains("无参 async sub")`，原子串保留即可）。

`codegen.rs` `gen_builtin_call`：

在 `(CallArg::SubRef(name_opt), ParamKind::SubRef)` 臂之后加

```rust
                (CallArg::SubRefArgs(name, exprs), ParamKind::SubRef) => {
                    b.push_task_ref(Some(self.name_to_ref[name]));
                    for e in exprs {
                        self.gen_expr(b, slots, e);
                    }
                    b.push_i(exprs.len() as i32); // argc（syscall 210 调用约定）
                }
```

并在 `(CallArg::SubRef(name_opt), ParamKind::SubRef)` 臂内，`push_task_ref` 之后对 `spawn_enemy` 追压 argc 0：

```rust
                (CallArg::SubRef(name_opt), ParamKind::SubRef) => {
                    match name_opt {
                        Some(name) => b.push_task_ref(Some(self.name_to_ref[name])),
                        None => b.push_task_ref(None),
                    }
                    if bi.name == "spawn_enemy" {
                        b.push_i(0); // argc = 0（syscall 210 调用约定，boss 换段刀）
                    }
                }
```

`TypedStmt::Spawn` 与 `gen_call` 里两处 `CallArg::XformRef(_) | CallArg::SubRef(_) => unreachable!` 改为 `CallArg::XformRef(_) | CallArg::SubRef(_) | CallArg::SubRefArgs(..) => unreachable!`。

`slots.rs` `call_args_depth`：

```rust
            // task 号 1 字 + 实参逐个驻留 + 末尾 argc 1 字（boss 换段刀）。
            CallArg::SubRefArgs(_, exprs) => {
                let mut inner_peak = 1usize;
                for (k, e) in exprs.iter().enumerate() {
                    inner_peak = inner_peak.max(1 + k + expr_depth(e));
                }
                (inner_peak.max(exprs.len() + 2), exprs.len() + 2)
            }
```

`crates/stg-ecl-compiler/src/lib.rs` builder `sys_spawn_enemy`：`self.push_task_ref(task_script);` 之后加 `self.push_i(0); // argc（210 调用约定，boss 换段刀）`，doc 注释补一句。

同步 `builtins.rs` 里 `spawn_enemy` 的 `doc`：在「task 为敌主任务 async sub 名或 none」后插入「,可写 name(实参…) 按签名带参(实参当帧求值拷进新任务 locals)」。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p stg-ecl-compiler 2>&1 | tail -5 && cargo test -p stg-core 2>&1 | tail -3`
Expected: 全绿（`committed_doc_segment_matches_generated` 若因 doc 改动红，是预期——Task 5 统一重跑 `gen-ecl-meta`；本步先跑 `cargo run -p stg-harness -- gen-ecl-meta` 让它绿）。

- [ ] **Step 7: 提交**

```bash
cargo run -p stg-harness -- gen-ecl-meta
cargo fmt --all
git add -A crates docs/ecl-lang/7-reference.md editors/vscode/stg-ecl/ecl-meta.json
git commit -m "feat(ecl): spawn_enemy task 位带参——syscall 210 调用约定追加实参+argc，OP_SPAWN/spawn_enemy 共用 write_args，typeck/codegen/slots 接 CallArg::SubRefArgs"
```

---

### Task 5: 表层内建七条 + `phase_begin` 糖 + `$self_enemy`

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（7 条 `Builtin` + `phase_begin` + `EngVarMeta` + 名单测试）
- Modify: `crates/stg-ecl-compiler/src/lang/ast.rs`（`EngVar::SelfEnemy`）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`（`phase_begin` 注入常量 + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（端到端测试）
- Regenerate: `docs/ecl-lang/7-reference.md`、`editors/vscode/stg-ecl/ecl-meta.json`

**Interfaces:**
- Consumes: Task 1–3 的 syscall 号与常量。
- Produces: 表层内建 `spell_result(slot: int) -> int`、`set_invuln(frames: int)`、`set_hitbox(r: fx)`、`set_hurtbox(r: fx)`、`set_enemy_flag(flag: int, on: int)`、`kill_all_enemies(mode: int)`、`clear_bullets_at(x: fx, y: fx, r: fx, stars: int)`、`phase_begin(slot: int, pattern: sub|none, time_limit: int, hp_threshold: int)`；`$self_enemy: int`。

- [ ] **Step 1: 写失败测试**

`codegen.rs` 测试模块：

```rust
    /// `phase_begin` 糖（boss 换段刀 spec §3.1）：与手写 `spell_begin(slot, 0, p, t, 0, SPELL_NONSPELL, thr)`
    /// 编译出逐字节相同的代码。
    #[test]
    fn phase_begin_lowers_byte_identical_to_spell_begin_with_nonspell_flag() {
        let sugar = "async sub p() { loop { wait(1); } }\n\
                     async sub boss() { phase_begin(0, p, 600, 300); wait_spell(); }\n\
                     sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 900, 0, 0, 0, boss); }";
        let hand = "async sub p() { loop { wait(1); } }\n\
                    async sub boss() { spell_begin(0, 0, p, 600, 0, SPELL_NONSPELL, 300); wait_spell(); }\n\
                    sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 900, 0, 0, 0, boss); }";
        let a = compile(sugar, "a.ecl").expect("糖应编译");
        let b = compile(hand, "b.ecl").expect("手写应编译");
        assert_eq!(a.code(), b.code());
    }
```

`lang/mod.rs` 端到端（同 Task 4 e2e 的写法）：

```rust
        /// 新内建端到端（boss 换段刀）：非符段超时 → spell_result==2 写进 globals；
        /// $self_enemy == spawn_enemy 返回值；set_invuln 等表层可调且零 fault。
        #[test]
        fn phase_begin_spell_result_and_self_enemy_end_to_end() {
            let src = "async sub p() { loop { wait(1); } }\n\
                       async sub boss() {\n\
                         set_global(22, $self_enemy);\n\
                         set_invuln(10); set_hitbox(20.0fx); set_hurtbox(24.0fx);\n\
                         set_enemy_flag(ENEMY_NO_BODY, 1);\n\
                         phase_begin(0, p, 5, 300);\n\
                         wait_spell();\n\
                         set_global(20, spell_result(0));\n\
                         clear_bullets_at(0.0fx, 100.0fx, 50.0fx, 0);\n\
                         kill_all_enemies(KILL_SILENT);\n\
                         loop { wait(1); }\n\
                       }\n\
                       sub main() {\n\
                         var b: int = spawn_enemy(0.0fx, 100.0fx, 900, 0, 0, 0, boss);\n\
                         set_global(21, b);\n\
                         loop { wait(1); }\n\
                       }";
            let img = compile(src, "t.ecl").expect("应编译成功");
            let t = stg_core::tables::TABLES_V0;
            let mut w = stg_core::step::World::new(1);
            w.start_main(&img).expect("main 应能派生");
            for f in 0..20 {
                stg_core::step::step(&mut w, &t, &img, &stg_core::input::InputFrame::empty(f));
            }
            let g = w.body.view().globals();
            assert_eq!(g[20], stg_core::spell::SPELL_END_TIMEOUT as i32, "非符段超时");
            assert_eq!(g[22], g[21], "$self_enemy == spawn_enemy 返回值");
            assert_eq!(w.body.view().diag().task_faults, 0);
        }
```

`builtins.rs` 测试：`lookup_finds_every_documented_builtin_by_name` 的 `names` 数组追加
`"spell_result", "set_invuln", "set_hitbox", "set_hurtbox", "set_enemy_flag", "kill_all_enemies", "clear_bullets_at", "phase_begin"`；
`void_builtins_have_none_return_type` 的数组追加 `"set_invuln", "set_hitbox", "set_hurtbox", "set_enemy_flag", "kill_all_enemies", "clear_bullets_at", "phase_begin"`；
`engine_var_table_covers_every_variant` 的穷尽 `match` 追加 `| EngVar::SelfEnemy`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-ecl-compiler phase_begin lookup_finds void_builtins engine_var_table 2>&1 | tail -10`
Expected: 失败/编译失败（内建与 `EngVar::SelfEnemy` 不存在）。

- [ ] **Step 3: 实现**

`builtins.rs` `BUILTINS`——符卡段（`spell_timer` 之后）：

```rust
    Builtin {
        name: "phase_begin",
        syscall: syscall::SYS_SPELL_BEGIN,
        is_op: false,
        // 糖（boss 换段刀 spec §3.1）：codegen 在第 1 位前注入 spell_id=0、在第 3 位后注入
        // bonus0=0 与 flags=SPELL_NONSPELL，降低为 spell_begin 7 参（见 `codegen::phase_begin_injects`）。
        params: &[Val(Int), Sub, Val(Int), Val(Int)],
        ret: None,
        doc: "开非符段:= spell_begin(slot, 0, pattern, time_limit, 0, SPELL_NONSPELL, hp_threshold);计时/血线/模式随段/血条照旧,不宣言不计 bonus 不出结算横幅,结束发 EVT_PHASE_ENDED;配 wait_spell() 与 spell_result(slot)",
        param_names: &["slot", "pattern", "time_limit", "hp_threshold"],
    },
    Builtin {
        name: "spell_result",
        syscall: syscall::SYS_SPELL_RESULT,
        is_op: false,
        params: &[Val(Int)],
        ret: Some(Int),
        doc: "槽 slot 最近一次结束方式:0 还没结束过 / SPELL_END_HP(1) 打到血线含 boss 死 / SPELL_END_TIMEOUT(2) 超时 / SPELL_END_MANUAL(3) spell_end;下一次 spell_begin 不清;越界返 0+计数",
        param_names: &["slot"],
    },
```

敌运动段之后（`set_anm_state` 之后）：

```rust
    // ── 敌判定族（syscall 440-443；boss 换段刀 2026-09-14）─────────────────────
    Builtin {
        name: "set_invuln",
        syscall: syscall::SYS_SET_INVULN,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "敌自身无敌 frames 帧(覆写;0 取消;期间自机弹不掉血不发命中事件,自机弹本就穿透不消耗);self 非 ENEMY → Fault;frames 越出 [0,65535] no-op+计数",
        param_names: &["frames"],
    },
    Builtin {
        name: "set_hitbox",
        syscall: syscall::SYS_SET_HITBOX,
        is_op: false,
        params: &[Val(Fx)],
        ret: None,
        doc: "敌自身体碰半径(撞自机那一圈,生成默认 12);钳 [0,1024]+计数;self 非 ENEMY → Fault。ZUN setHitbox(w,h) 的 w 是直径还是半径待验",
        param_names: &["r"],
    },
    Builtin {
        name: "set_hurtbox",
        syscall: syscall::SYS_SET_HURTBOX,
        is_op: false,
        params: &[Val(Fx)],
        ret: None,
        doc: "敌自身受击半径(被自机弹/伤害区打中那一圈,生成默认 16);钳 [0,1024]+计数;self 非 ENEMY → Fault",
        param_names: &["r"],
    },
    Builtin {
        name: "set_enemy_flag",
        syscall: syscall::SYS_SET_ENEMY_FLAG,
        is_op: false,
        params: &[Val(Int), Val(Int)],
        ret: None,
        doc: "置(on!=0)/清敌自身标志:flag 为 ENEMY_NO_BODY(不体碰,仍吃弹) / ENEMY_KILLALL_EXEMPT(kill_all_enemies 不杀) 的非空组合;其它位 no-op+计数;self 非 ENEMY → Fault",
        param_names: &["flag", "on"],
    },
```

死亡效果段（`die` 之后）：

```rust
    Builtin {
        name: "kill_all_enemies",
        syscall: syscall::SYS_KILL_ALL_ENEMIES,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "清场:按池序杀除调用者自己/带 ENEMY_KILLALL_EXEMPT/已在死之外的全部敌;mode KILL_SILENT(0) 静默退场(不掉不加分无事件) / KILL_DIE(1) 同 die() 全套;其它 mode no-op+计数;owner 无限制",
        param_names: &["mode"],
    },
```

清弹段（`clear_bullets` 之后）：

```rust
    Builtin {
        name: "clear_bullets_at",
        syscall: syscall::SYS_CLEAR_BULLETS_AT,
        is_op: false,
        params: &[Val(Fx), Val(Fx), Val(Fx), Val(Int)],
        ret: None,
        doc: "圆形清弹:以 (x,y) 为心、半径 r 铺存活 1 帧的清弹区;stars=0 不转星星,非 0 同 clear_bullets 转星;扩张消弹波就每帧调一次加大 r;owner 无限制",
        param_names: &["x", "y", "r", "stars"],
    },
```

`ENGINE_VARS` 末尾：

```rust
    EngVarMeta {
        ev: EngVar::SelfEnemy,
        name: "self_enemy",
        ty: Int,
        syscall: syscall::SYS_SELF_ENEMY,
        doc: "任务 owner 敌的敌号(与 spawn_enemy 返回值同编码,可喂 enemy_alive/enemy_x 等);owner 不是敌 → -1(**不是 0**:敌号 0 合法)",
    },
```

`ast.rs` `EngVar` 枚举追加 `SelfEnemy,`（照邻居写一行 doc）。

`codegen.rs`：在 `emits_wait_one_after` 之后加

```rust
/// `phase_begin` 糖的常量注入（boss 换段刀 spec §3.1）：返回「在表层第 `i` 位**之前**要追压的立即数」。
/// 表层 `(slot, pattern, time_limit, hp_threshold)` → 字节码 `spell_begin` 7 参
/// `(slot, spell_id=0, pattern, time_limit, bonus0=0, flags=SPELL_NONSPELL, hp_threshold)`。
fn phase_begin_injects(name: &str, i: usize) -> &'static [i32] {
    const NONSPELL: i32 = stg_core::spell::SPELL_NONSPELL as i32;
    match (name, i) {
        ("phase_begin", 1) => &[0],
        ("phase_begin", 3) => &[0, NONSPELL],
        _ => &[],
    }
}
```

`gen_builtin_call` 的 `while` 循环体里，`let (a, pk) = (&args[i], &bi.params[i]);` 之前加：

```rust
            for &v in phase_begin_injects(bi.name, i) {
                b.push_i(v);
            }
```

- [ ] **Step 4: 重新生成元数据并跑测试**

Run: `cargo run -p stg-harness -- gen-ecl-meta && cargo test -p stg-ecl-compiler 2>&1 | tail -5 && cargo test -p stg-harness 2>&1 | tail -3`
Expected: 全绿（`committed_doc_segment_matches_generated` 绿；手册 ```ecl 围栏编译全绿）。
若 `phase_begin` 字节比较失败，检查 `SPELL_NONSPELL` 常量引用与字面量 `4` 是否都降低为同一条 `PUSHI`（常量折叠）——两者应一致。

- [ ] **Step 5: 提交**

```bash
cargo fmt --all
git add -A crates docs/ecl-lang/7-reference.md editors/vscode/stg-ecl/ecl-meta.json
git commit -m "feat(ecl): 表层内建 spell_result/set_invuln/set_hitbox/set_hurtbox/set_enemy_flag/kill_all_enemies/clear_bullets_at + phase_begin 糖 + \$self_enemy"
```

---

### Task 6: `ENGINE_VER` 21、桥与 HUD、全闸门

**Files:**
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER` 20→21 + 理由段）
- Modify: `crates/stg-godot/src/bridge.rs`（导出 `EVT_PHASE_ENDED`、`SPELL_NONSPELL`）
- Modify: `godot/scripts/hud.gd`（非符段只显示倒计时）

**Interfaces:**
- Consumes: 全部前序 Task。

- [ ] **Step 1: bump 与理由**

`lib.rs`，在 `**19 → 20**` 段之后、`pub const ENGINE_VER` 之前加：

```rust
/// **20 → 21**（boss 换段与敌人钩子刀，2026-09-14）：**号表 + 调用约定 + 布局 + 碰撞矩阵 + 事件号 + 结算行为**。
/// ① syscall 新增 `026 self_enemy` / `131 spell_result` / `440–443` 敌判定族 / `531 kill_all_enemies` /
/// `541 clear_bullets_at`；② `210 spawn_enemy` 调用约定追加「实参…, argc」；③ `WorldBody.spell_last_result: [u8; 2]`
/// 进校验和与存档（对齐可能吞掉、D20，已直测）；④ 碰撞行 3 跳过 `ENEMY_NO_BODY`；⑤ `EVT_PHASE_ENDED = 12`；
/// ⑥ 符卡结算：`SPELL_NONSPELL` 非符段、所有超时把绑定 boss 的 hp 钉到血线、超时自动清弹不转星星（`FIELD_NO_STAR`）。
/// 金向量：风铃卡时限 3600 > 600 帧窗口，行为不走超时；新字段恒 0 但进哈希 ⇒ 预期改变，实测为准。
pub const ENGINE_VER: u32 = 21;
```

- [ ] **Step 2: 桥常量与 HUD**

`bridge.rs` 在 `EVT_SHOT_HIT_ENEMY` 常量之后：

```rust
    #[constant]
    const EVT_PHASE_ENDED: i64 = stg_core::events::EVT_PHASE_ENDED as i64;
    /// 符卡槽 flags 的非符段位（boss 换段刀）：HUD 据此不显示卡名。
    #[constant]
    const SPELL_NONSPELL: i64 = stg_core::spell::SPELL_NONSPELL as i64;
```

`hud.gd` 卡名那段改为：

```gdscript
		var s := bridge.hud_spell(0)
		if not s.is_empty() and int(s["active"]) == 1:
			var secs := int(s["frames_left"]) / 60
			if int(s["flags"]) & WorldBridge.SPELL_NONSPELL:
				spell_l.text = "%d" % secs
			else:
				var sname: String = ContentTables.SPELL_NAMES.get(int(s["spell_id"]), "Spell #%d" % int(s["spell_id"]))
				spell_l.text = "%s  %d" % [sname, secs]
		else:
			spell_l.text = ""
```

- [ ] **Step 3: 全闸门**

Run（逐条，全部须成功）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked" | tail -20
cargo run --release -p stg-harness -- storm
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- check godot/ecl/game
cargo build -p stg-godot && bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
cargo run -p stg-harness -- golden --out /tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/c9f33042-31db-4d46-b93b-bfcadf69c028/scratchpad/golden21.txt && md5sum /tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/c9f33042-31db-4d46-b93b-bfcadf69c028/scratchpad/golden21.txt
```

Expected: 全绿；两冒烟 `SMOKE OK`；记下新金向量 md5（与 20 版 `15a5167c…` 比较，写进 Task 7 的 PROGRESS 行与 lib.rs 理由段「实测」）。

- [ ] **Step 4: 提交**

```bash
git add -A crates/stg-core/src/lib.rs crates/stg-godot godot/scripts/hud.gd
git commit -m "feat: ENGINE_VER 21（boss 换段与敌人钩子刀）+ 桥导出 EVT_PHASE_ENDED/SPELL_NONSPELL + HUD 非符段只显示倒计时"
```

---

### Task 7: 文档同步与收口

**Files:**
- Modify: `docs/ecl-lang/3-enemy.md`、`docs/ecl-lang/4-bullets.md`、`docs/ecl-lang/6-spell-and-stage.md`、`docs/ecl-lang/7-reference.md`（手写的引擎常量表）
- Modify: `docs/ecl-ops.md`、`docs/zun-ecl-v2-reference.md`、`stg-world-design.md`、`docs/follow-ups.md`
- Modify: `docs/superpowers/specs/2026-09-14-boss-phase-enemy-hooks-design.md`（状态行）、`PROGRESS.md`

- [ ] **Step 1: 手册**（每个新增 ```ecl 围栏都会被 `cargo test -p stg-harness` 真编译）

- `3-enemy.md`：
  - 「敌的生成与轮询」节加小节「给敌任务传参」：写法 `spawn_enemy(..., zako(dir, 2.5fx))`、实参当帧求值拷贝、次帧首跑；为什么别用 globals 顶替（同帧两只读到同一值）；`fire`/`sh_task`/`spell_begin` 仍只收无参 sub。附一段可编译示例（左右对称编队传 `dir`）。
  - 新节「判定与无敌」：`set_invuln` / `set_hitbox` / `set_hurtbox` / `set_enemy_flag(ENEMY_NO_BODY | ENEMY_KILLALL_EXEMPT, on)`；说明自机弹命中本就穿透不消耗、invuln 期间不掉血不发命中事件；`ENEMY_NO_BODY` 仍吃弹。
  - 「三条死亡路径对照」表加两行：`kill_all_enemies(KILL_SILENT)`（✘✘✘，hp 不动）、`kill_all_enemies(KILL_DIE)`（✔✔✔，同 die）；文字说明跳过调用者/免清/已死，杀绑定 boss 会按血线结算当前段。
  - 「数学与查询」节加 `$self_enemy`：非敌 -1 不是 0 的理由；使魔拿 boss 号的示例（boss 用 `spawn_enemy(..., familiar($self_enemy))`）。
- `4-bullets.md`：`clear_bullets` 旁加 `clear_bullets_at(x, y, r, stars)`：一帧圆形区、`stars=0` 不转星、扩张消弹波写法（`for r in` 每帧 `clear_bullets_at` + `wait(1)`）。
- `6-spell-and-stage.md` 符卡节：
  - 参数表 `flags` 行补 bit2 `SPELL_NONSPELL`，并注明三个位现在都有注入常量。
  - 新小节「非符段：`phase_begin`」：与 `spell_begin` 的异同表、`wait_spell()` 通用、HUD 不显示卡名。
  - 新小节「超时发生了什么」：钉血（`hp = min(hp, 血线)`，剩血不漏段）、超时清弹不转星、普通卡 FAILED/耐久卡看资格/非符段只发 `EVT_PHASE_ENDED`。
  - 新小节「`spell_result(slot)`」：取值表 + 「超时不给奖励」示例（即 spec §3.3 那段）。
  - 「三条原语加一条糖」改为「四条原语加两条糖」并更新列表。
- `7-reference.md` 引擎常量表加行：`SPELL_SURVIVAL/NO_CLEAR/NONSPELL`（1/2/4）、`SPELL_END_HP/TIMEOUT/MANUAL`（1/2/3）、`ENEMY_NO_BODY/ENEMY_KILLALL_EXEMPT`（2/4）、`KILL_SILENT/KILL_DIE`（0/1）。
- `docs/ecl-lang.md` 索引表不改（篇目未变）。

- [ ] **Step 2: 字节码与设计文档**

- `ecl-ops.md`：0xx 表加 026；1xx 加 131；2xx 的 210 行改写压栈序与门禁（argc 超限/栈不够 → Fault(2)；形参个数不符、none 带参 → Fault(0)；敌不建）；4xx 加 440–443；5xx 加 531、541；「符卡计器」节补非符段、超时钉血、超时不给星、`spell_last_result`、事件 12。
- `zun-ecl-v2-reference.md`：文末新增「## 游戏指令迁移对照（boss 换段刀 2026-09-14）」表，逐行：
  `setInterrupt(slot,hp,t,sub)` + `$TIMEOUT` → `phase_begin`/`spell_begin` + `wait_spell` + `spell_result`（结构差异：ZUN 抢占式跳转、我们顺序编排）；
  `setInvuln` → `set_invuln`；`setHitbox`/`setHurtbox` → `set_hitbox`/`set_hurtbox`（w/h 直径还是半径待验）；
  `flagSet(2)` → `set_enemy_flag(ENEMY_NO_BODY, 1)`；`flagSet(32)` → `ENEMY_KILLALL_EXEMPT`；
  `enmKillAll` → `kill_all_enemies`；`$ID` → `$self_enemy`；`$I0–3/%F0–7` 继承 → `spawn_enemy(..., sub(实参…))`；
  `etCancel(640)` → `clear_bullets()`；`etClear(640)` → `clear_bullets_at(0.0fx, 224.0fx, 400.0fx, 0)`；`etCancel(r)` 扩张波 → 每帧 `clear_bullets_at`；
  迁移坑：`movePosTime(0,0,0,0)` 是取消插值不是瞬移（写 `move_to(0, $self_x, $self_y, 0)`）。
- `stg-world-design.md`：D8 碰撞矩阵行 3 注记「`ENEMY_NO_BODY` 跳过（2026-09-14）」；D12 syscall 表补 7 行。

- [ ] **Step 3: follow-ups、spec、PROGRESS**

- `docs/follow-ups.md` D 段追加一条（编号取现有最大 D 号 +1）：「boss 换段刀的五条非目标」——逐条抄 spec §10 的 1–5 并各带触发点。
- spec 状态行改为「**已落地（2026-09-14）**」；若实施中有偏离 spec 的地方，在文末加「## 11. 实施偏差」表。
- `PROGRESS.md`：「现在」段重写（位置 = 本刀落地、`ENGINE_VER` 21、金向量新 md5、实证测试数、下一步 = 第 1 关内容刀）；里程碑史表首行插一行（照 2026-09-14 玩法刀那行的密度写：核/编译器/桥/壳各做了什么 + 闸门结果）。

- [ ] **Step 4: 验证文档围栏与提交**

Run: `cargo test -p stg-harness 2>&1 | tail -3`
Expected: 全绿（手册围栏全部编译通过、生成段防漂移绿）。

```bash
git add -A docs PROGRESS.md stg-world-design.md
git commit -m "docs: boss 换段与敌人钩子刀收口——ecl 手册 3/4/6/7、ecl-ops、ZUN 迁移对照、world-design D8/D12、follow-ups 非目标、spec 已落地、PROGRESS"
```
