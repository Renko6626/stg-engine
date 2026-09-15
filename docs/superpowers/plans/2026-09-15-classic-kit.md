# 经典机体刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (inline；用户定：任务间审阅从简，不派逐任务复审子 agent). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `WorldTables` 里加按机体分派的规则套件 `Kit { Chronos, Classic(BombCfg) }`，新增机体 1（经典 bomb + 场底重生 + 无跳躍）供 RL 训练，机体 0（東方時環晷）逐字节不变。

**Architecture:** 数据先行：`CharacterCfg.kit` + 捞回的 `BombCfg` 族 + 表字节 v6；行为在 `world/player.rs` 三个分派点（A 组 X/C 键、C 组 `commit_death`）穷尽 `match`。新 `PlayerState.bomb_timer` 是 bomb 进行中的单一真相源。step 顺序、碰撞矩阵、词表、号表都不动。

**Tech Stack:** Rust 1.94（stg-core / stg-harness / stg-ecl-compiler）。

**Spec:** `docs/superpowers/specs/2026-09-15-classic-kit-design.md`

## Global Constraints

- stg-core 断层线：不引入浮点 / 时钟 / 宿主 RNG / 无序容器（CLAUDE.md I1–I7）。
- `TABLE_VERSION` 5 → 6；`ENGINE_VER` 21 → 22（Task 4 一次 bump）。
- 机体 0 = `Kit::Chronos`，行为逐字节不变；机体 1 = 机体 0 移速/判定/shot + `Kit::Classic(BombCfg{frames:120, invuln:120, attract_items:true, fields:[全屏消弹(FieldCenter, FIELD_RADIUS_FULLSCREEN, FIELD_CLEAR_BULLETS, 0, 120), 伤害圆(PlayerAtCast, 120, FIELD_DAMAGE, 4, 120)]})`。
- Classic 场底重生位置 `(0, 384)`，无敌 `RESPAWN_INVULN`(120)，**不发** `EVT_REWIND_REQUESTED`；不新增生命态（退役值 3 不复用）。
- `STOP_STOCK_MAX` 两套件共用，名字不改。
- 清理临时/变异代码**只用编辑撤回，禁止 `git checkout <file>` / `git stash`**。金向量对拍 base 用 `git worktree add` 一次性副本。
- commit 结尾：`Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`。
- 每个 Task 结束 `cargo test --workspace` 全绿再提交。

## File Map

| 文件 | 本刀改动 |
|---|---|
| `crates/stg-core/src/tables.rs` | `Kit`/`BombCfg`/`BombField`/`BombOrigin`；`characters: Box<[CharacterCfg]>`；validate；字节 v6；v0 机体 1；测试 |
| `crates/stg-core/src/tables/tables_v0.bin` | `bake-tables` 重烘 |
| `crates/stg-core/src/player.rs` | `PlayerState.bomb_timer`；`RESPAWN_INVULN` 文档 |
| `crates/stg-core/src/world/player.rs` | `try_bomb`；A 组分派；C 组 `bomb_timer`；`commit_death(i, tables)` 分派；测试 |
| `crates/stg-core/src/timeline.rs` | 测试：Classic 死亡不遡行 |
| `crates/stg-core/src/lib.rs` / `step.rs` | `ENGINE_VER` 22 文档 + 锚定测试；尺寸哨兵账 |
| `crates/stg-ecl-compiler/src/lang/builtins.rs` | `add_bombs` 文案 → `gen-ecl-meta` |
| 文档 | `stg-world-design.md` D6、`docs/architecture.md`、`docs/ecl-ops.md` 511 行、`docs/ecl-lang/6-spell-and-stage.md`、`docs/follow-ups.md` D22、`PROGRESS.md`、`CLAUDE.md` |

---

### Task 1: 表数据层——`Kit` + 机体 1 + 字节 v6

**Files:**
- Modify: `crates/stg-core/src/tables.rs`
- Modify: `crates/stg-core/src/tables/tables_v0.bin`（重烘）

**Interfaces:**
- Produces: `pub enum Kit { Chronos, Classic(BombCfg) }`；`pub struct BombCfg { frames: u16, invuln: u16, attract_items: bool, fields: Box<[BombField]> }`；`pub struct BombField { origin: BombOrigin, radius: Fx, flags: u8, dmg_per_frame: u16, life: u16 }`；`pub enum BombOrigin { FieldCenter, PlayerAtCast }`；`CharacterCfg.kit: Kit`；`WorldTables.characters: Box<[CharacterCfg]>`（`TABLES_V0.characters.len() == 2`）。

- [ ] **Step 1: 写失败测试**（`tables.rs` `mod tests` 末尾）

```rust
    /// v0 机体 1 = 机体 0 的移速/判定/shot + Classic；bomb 数值钉旧 v0（改内容要有意识地改本测试）。
    #[test]
    fn v0_character_1_is_character_0_plus_classic_kit() {
        let (c0, c1) = (&TABLES_V0.characters[0], &TABLES_V0.characters[1]);
        assert_eq!(TABLES_V0.characters.len(), 2);
        assert_eq!(c0.kit, Kit::Chronos);
        assert_eq!(
            (c1.high_speed, c1.low_speed, c1.inv_sqrt2, c1.hit_radius, c1.graze_radius),
            (c0.high_speed, c0.low_speed, c0.inv_sqrt2, c0.hit_radius, c0.graze_radius)
        );
        assert_eq!(c1.shot, c0.shot);
        let Kit::Classic(b) = &c1.kit else { panic!("机体 1 必须是 Classic") };
        assert_eq!((b.frames, b.invuln, b.attract_items), (120, 120, true));
        assert_eq!(b.fields.len(), 2);
        assert_eq!(b.fields[0].origin, BombOrigin::FieldCenter);
        assert_eq!(b.fields[0].radius, crate::field::FIELD_RADIUS_FULLSCREEN);
        assert_eq!(b.fields[0].flags, crate::field::FIELD_CLEAR_BULLETS);
        assert_eq!(b.fields[1].origin, BombOrigin::PlayerAtCast);
        assert_eq!(b.fields[1].radius, Fx::from_int(120));
        assert_eq!(b.fields[1].flags, crate::field::FIELD_DAMAGE);
        assert_eq!(b.fields[1].dmg_per_frame, 4);
        assert!(b.fields.iter().all(|f| f.life == b.frames));
    }

    /// 往返：Classic 段逐字段相等。判别力：漏写/错序任一字段都红。互异非零值（S1 纪律）。
    #[test]
    fn classic_kit_survives_a_bytes_roundtrip() {
        let mut t = build_tables_v0();
        t.characters[1].kit = Kit::Classic(BombCfg {
            frames: 77,
            invuln: 91,
            attract_items: false,
            fields: Box::new([
                BombField { origin: BombOrigin::PlayerAtCast, radius: Fx::from_int(33), flags: crate::field::FIELD_DAMAGE, dmg_per_frame: 9, life: 5 },
                BombField { origin: BombOrigin::FieldCenter, radius: Fx::from_int(44), flags: crate::field::FIELD_CLEAR_BULLETS, dmg_per_frame: 0, life: 6 },
                BombField { origin: BombOrigin::PlayerAtCast, radius: Fx::from_int(55), flags: crate::field::FIELD_CLEAR_BULLETS | crate::field::FIELD_DAMAGE, dmg_per_frame: 2, life: 7 },
            ]),
        });
        let back = WorldTables::from_bytes(&t.to_bytes()).expect("往返应成功");
        assert_eq!(back.characters, t.characters);
    }

    /// validate：Classic 四条坏行各自被拒（radius 越界 / 未定义 flags 位 / frames==0 / life==0）；
    /// 空角色表被拒。只测一条的话其余三条漏写也绿。
    #[test]
    fn classic_kit_validate_rejects_bad_rows() {
        let mk = |mutate: &dyn Fn(&mut BombCfg)| {
            let mut t = build_tables_v0();
            let Kit::Classic(b) = &mut t.characters[1].kit else { unreachable!() };
            mutate(b);
            t
        };
        assert!(!mk(&|b| b.fields[0].radius = Fx::from_int(-1)).validate(), "radius 越界须拒");
        assert!(!mk(&|b| b.fields[0].flags = 0x80).validate(), "未定义 flags 位须拒");
        assert!(!mk(&|b| b.frames = 0).validate(), "frames==0 须拒");
        assert!(!mk(&|b| b.fields[0].life = 0).validate(), "life==0 须拒");
        let mut empty = build_tables_v0();
        empty.characters = Box::new([]);
        assert!(!empty.validate(), "空角色表须拒");
        assert!(build_tables_v0().validate(), "内建表本身必须合法");
    }

    /// 造一份「哈希自洽但某字节非法」的表：`from_bytes` 的 FNV 自校先于解析，必须重算回填。
    fn tamper(mut bytes: Vec<u8>, at: usize, v: u8) -> Vec<u8> {
        use crate::checksum::Fnv1a64;
        bytes[at] = v;
        let mut h = Fnv1a64::new();
        h.write_bytes(&bytes[TABLE_HEADER..]);
        bytes[8..TABLE_HEADER].copy_from_slice(&h.finish().to_le_bytes());
        bytes
    }

    /// 唯一定位一段字节模式（模式若不唯一当场红，别悄悄改错地方）。
    fn find_unique(bytes: &[u8], pat: &[u8]) -> usize {
        let hits: Vec<usize> = bytes.windows(pat.len()).enumerate().filter(|(_, w)| *w == pat).map(|(i, _)| i).collect();
        assert_eq!(hits.len(), 1, "定位模式必须唯一");
        hits[0]
    }

    /// 三个判别值字节各自被拒，不静默变默认值。定位：机体 1 的 Classic 段头 =
    /// `kit_tag=1` ⧺ `frames=120 (78 00)` ⧺ `invuln=120 (78 00)` ⧺ `attract=1` ⧺ `n_fields=2 (02 00 00 00)`
    /// ⧺ 首条 `origin=0`。
    #[test]
    fn classic_kit_rejects_unknown_discriminants() {
        const HEAD: [u8; 11] = [0x01, 0x78, 0x00, 0x78, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00];
        let base = build_tables_v0().to_bytes();
        let at = find_unique(&base, &HEAD);
        for (off, field) in [(0usize, "kit"), (5, "attract_items"), (10, "bomb_origin")] {
            match WorldTables::from_bytes(&tamper(base.clone(), at + off, 7)) {
                Err(TableLoadError::BadDiscriminant { field: f, value }) => assert_eq!((f, value), (field, 7)),
                other => panic!("{field} 坏判别值须被拒，实得 {other:?}"),
            }
        }
    }

    /// 角色计数 0 → ArityMismatch（空表不得开机）。直接序列化一份 `characters = []` 的表
    /// （`to_bytes` 不跑 validate，哈希自洽），解析在 validate 之前就该拒。
    #[test]
    fn zero_characters_is_an_arity_error() {
        let mut t = build_tables_v0();
        t.characters = Box::new([]);
        match WorldTables::from_bytes(&t.to_bytes()) {
            Err(TableLoadError::ArityMismatch { field: "characters", .. }) => {}
            other => panic!("空角色表须 ArityMismatch，实得 {other:?}"),
        }
    }
```

- [ ] **Step 2: 跑测试确认编译失败**

Run: `cargo test -p stg-core --lib tables:: 2>&1 | tail -20`
Expected: 编译错误 `cannot find type Kit` / `BombCfg`。

- [ ] **Step 3: 实现**

1. 在 `ShotTypeCfg` 定义之前加类型（文档注释从 `633c3f1^:crates/stg-core/src/tables.rs` 第 64–116 行捞回，改一句「住在 `Kit::Classic` 里」）：

```rust
/// 机体规则套件（经典机体刀 2026-09-15）：X 键 / C 键 / 死亡三处行为打包成一个值。
/// 分派点（`world/player.rs` 的 A 组按键、`commit_death`）一律穷尽 `match`——加变体忘了
/// 处理任何一处 ⇒ 编译不过（D18 手法）。打包而非三个正交字段：可用组合只有两种（YAGNI）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kit {
    /// 東方時環晷：X = 停止，C = 跳躍，死亡 = 原地继续 + `EVT_REWIND_REQUESTED`。
    Chronos,
    /// 东方原作语义（RL 训练机体）：X = bomb，C = 无，死亡 = 场底重生。
    Classic(BombCfg),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BombCfg {
    /// 效果时长（帧）= `PlayerState.bomb_timer` 初值。
    pub frames: u16,
    /// 无敌帧。**允许 > `frames`**：那正是"防炸完立刻死"的旋钮。
    pub invuln: u16,
    /// 起爆当帧是否全屏吸道具。
    pub attract_items: bool,
    /// 起爆时铺的作用区，**按声明序**（I4）。合法可空（"只给无敌"）。
    pub fields: Box<[BombField]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BombField {
    pub origin: BombOrigin,
    pub radius: Fx,
    /// `FIELD_CLEAR_BULLETS` | `FIELD_DAMAGE` 的组合。
    pub flags: u8,
    pub dmg_per_frame: u16,
    pub life: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BombOrigin {
    /// 场心 `(0, FIELD_HEIGHT/2)`（全屏效果用）。
    FieldCenter,
    /// **起爆那一帧**的自机位置，之后不动。
    PlayerAtCast,
}
```

2. `CharacterCfg` 末尾加 `pub kit: Kit,`（文档：「规则套件；机体 0 Chronos / 机体 1 Classic」）。
3. `WorldTables.characters` 改 `pub characters: Box<[CharacterCfg]>,`，文档改为「v0 两个机体（0 = 時環晷咲夜 Chronos，1 = 经典训练机体 Classic）；≥1 条，`validate` 押」。
4. `build_tables_v0`：把现有 `characters: [CharacterCfg { … shot }]` 改为先绑 `let c0 = CharacterCfg { …, shot, kit: Kit::Chronos };`，再

```rust
    let c1 = CharacterCfg {
        kit: Kit::Classic(BombCfg {
            frames: 120,
            invuln: 120,
            attract_items: true,
            fields: Box::new([
                // ① 全屏消弹：life = frames ⇒ 整段期间逐帧消掉新飞进来的弹（保护时长天然成立）。
                BombField { origin: BombOrigin::FieldCenter, radius: crate::field::FIELD_RADIUS_FULLSCREEN, flags: crate::field::FIELD_CLEAR_BULLETS, dmg_per_frame: 0, life: 120 },
                // ② 起爆点伤害圆（不跟随）：120 帧 × 4 ≈ 480 伤害。
                BombField { origin: BombOrigin::PlayerAtCast, radius: Fx::from_int(120), flags: crate::field::FIELD_DAMAGE, dmg_per_frame: 4, life: 120 },
            ]),
        }),
        ..c0.clone()
    };
```

   `characters: Box::new([c0, c1]),`。
5. `validate`：循环前加 `if self.characters.is_empty() { return false; }`；`for c in self.characters.iter()` 循环体末尾加

```rust
            if let Kit::Classic(b) = &c.kit {
                if b.frames == 0 {
                    return false;
                }
                for f in b.fields.iter() {
                    if f.life == 0
                        || f.flags & !(crate::field::FIELD_CLEAR_BULLETS | crate::field::FIELD_DAMAGE) != 0
                        || !radius_in_range(f.radius)
                    {
                        return false;
                    }
                }
            }
```

6. `TABLE_VERSION` 改 `6`。`to_bytes` 每个角色写完 `option_pos` 后追加：

```rust
            // kit 段（经典机体刀，v6）：tag + Classic 载荷。写入顺序 = 读出顺序。
            match &c.kit {
                Kit::Chronos => out.push(0),
                Kit::Classic(b) => {
                    out.push(1);
                    out.extend_from_slice(&b.frames.to_le_bytes());
                    out.extend_from_slice(&b.invuln.to_le_bytes());
                    out.push(u8::from(b.attract_items));
                    out.extend_from_slice(&(b.fields.len() as u32).to_le_bytes());
                    for f in b.fields.iter() {
                        out.push(match f.origin {
                            BombOrigin::FieldCenter => 0,
                            BombOrigin::PlayerAtCast => 1,
                        });
                        out.extend_from_slice(&f.radius.raw().to_le_bytes());
                        out.push(f.flags);
                        out.extend_from_slice(&f.dmg_per_frame.to_le_bytes());
                        out.extend_from_slice(&f.life.to_le_bytes());
                    }
                }
            }
```

7. `from_bytes`：把 `if nc != 1 { … }` 改为 `if nc == 0 { return Err(ArityMismatch { field: "characters", expected: 1, actual: 0 }) }`，把单个角色的读取包进 `let mut characters = Vec::with_capacity(nc); for _ in 0..nc { … characters.push(CharacterCfg { …, kit }); }`，其中 `kit` 由新函数读：

```rust
/// 读 kit 段（v6）。坏判别字节一律拒（`BadDiscriminant`），不得静默变默认值。
fn read_kit(r: &mut Reader) -> Result<Kit, TableLoadError> {
    let tag = r.u8()?;
    match tag {
        0 => Ok(Kit::Chronos),
        1 => {
            let frames = r.u16()?;
            let invuln = r.u16()?;
            let attract_items = match r.u8()? {
                0 => false,
                1 => true,
                v => return Err(TableLoadError::BadDiscriminant { field: "attract_items", value: v }),
            };
            let n = r.u32()? as usize;
            let mut fields = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                let origin = match r.u8()? {
                    0 => BombOrigin::FieldCenter,
                    1 => BombOrigin::PlayerAtCast,
                    v => return Err(TableLoadError::BadDiscriminant { field: "bomb_origin", value: v }),
                };
                fields.push(BombField { origin, radius: r.fx()?, flags: r.u8()?, dmg_per_frame: r.u16()?, life: r.u16()? });
            }
            Ok(Kit::Classic(BombCfg { frames, invuln, attract_items, fields: fields.into_boxed_slice() }))
        }
        v => Err(TableLoadError::BadDiscriminant { field: "kit", value: v }),
    }
}
```

   `WorldTables { characters: characters.into_boxed_slice(), … }`。`TableLoadError::BadDiscriminant` 文档里的例子改成 `kit` / `bomb_origin`。
8. `Vec::with_capacity(nc)`：`nc` 来自外部字节，用 `nc.min(16)` 防巨量预分配。
9. 全仓修编译：`grep -rn "characters: \[" crates` 里的测试字面量改 `Box::new([...])`；tables 测试里 `bad_version` 相关若按版本号字面量写要跟 `TABLE_VERSION`。

- [ ] **Step 4: 重烘表并跑测试**

Run: `cargo run -p stg-harness -- bake-tables && cargo run -p stg-harness -- verify-tables && cargo test -p stg-core --lib tables:: 2>&1 | tail -5`
Expected: `verify-tables: 全部表与 commit 字节一致 ✔`；tables 测试全过。

- [ ] **Step 5: 全量测试**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked" | head -20`
Expected: 全 `ok`。若有测试钉死表 `content_hash` 或 `tables_v0.bin` 字节长度，改成新实测值并在断言信息里注明「经典机体刀 v6」。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-core/src/tables.rs crates/stg-core/src/tables/tables_v0.bin
git commit -m "feat(core): 经典机体刀 T1——WorldTables 规则套件 Kit（Chronos/Classic+BombCfg 族捞回）+ 机体 1 + 表字节 v6 重烘"
```

---

### Task 2: 经典 bomb——`bomb_timer` + `try_bomb` + X 键分派

**Files:**
- Modify: `crates/stg-core/src/player.rs`（`PlayerState.bomb_timer`、`spawn` 初值）
- Modify: `crates/stg-core/src/world/player.rs`

**Interfaces:**
- Consumes: Task 1 的 `Kit` / `BombCfg` / `BombOrigin`，`TABLES_V0.characters[1]`。
- Produces: `PlayerState.bomb_timer: u16`；`WorldBody::try_bomb(&mut self, i: usize, cfg: &BombCfg)`（私有）；测试辅助 `classic_world() -> Box<World>`、`press_k(w, buttons)`。

- [ ] **Step 1: 写失败测试**（`world/player.rs` `mod tests` 末尾新段）

```rust
    // ── 经典机体（经典机体刀 2026-09-15）：机体 1 = Kit::Classic ────────────────

    /// 机体 1 世界：自机换成 `spawn(1, …)`，其余同 `World::new`。
    fn classic_world() -> Box<crate::step::World> {
        let mut w = crate::step::World::new(1);
        w.body.players[0] =
            crate::player::PlayerState::spawn(1, &crate::tables::TABLES_V0.characters[1]);
        w
    }

    fn classic_bomb() -> &'static crate::tables::BombCfg {
        match &crate::tables::TABLES_V0.characters[1].kit {
            crate::tables::Kit::Classic(b) => b,
            crate::tables::Kit::Chronos => unreachable!("机体 1 必须是 Classic"),
        }
    }

    /// ①② 成对：窗口内能救且不扣命；窗口耗尽后按 X 救不回、不扣 bomb。
    #[test]
    fn classic_deathbomb_inside_the_window_revives_without_costing_a_life() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = crate::player::DEATHBOMB_WINDOW;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_ALIVE);
        assert_eq!(w.body.players[0].lives, lives0, "决死救人不扣命");
        assert_eq!(w.body.players[0].state_timer, 0);
        assert_eq!(w.body.players[0].bombs, 0);
        assert_eq!(w.body.players[0].bomb_timer, classic_bomb().frames);
    }

    #[test]
    fn classic_bomb_after_the_window_closed_cannot_undo_the_death() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0); // 窗口耗尽 → commit_death
        assert_eq!(w.body.players[0].lives, lives0 - 1, "已经扣命");
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].lives, lives0 - 1, "命不会退");
        assert_eq!(w.body.players[0].deaths, 1);
    }

    /// 伤害圆几何判别：圈内敌掉血、圈外不掉；圆心 = 起爆点 (0,200) ≠ 场心 (0,224)。
    #[test]
    fn classic_bomb_damage_field_is_at_the_cast_point_and_hits_only_inside() {
        use crate::math::Fx;
        let mut w = classic_world();
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(200);
        w.body.players[0].bombs = 1;
        let near = crate::world::test_support::spawn_enemy(&mut w, 0, 240, 1000); // 距 40
        let far = crate::world::test_support::spawn_enemy(&mut w, 0, 40, 1000); // 距 160 > 120+16
        let (ni, fi) = (w.body.enemies.get(near).unwrap(), w.body.enemies.get(far).unwrap());
        let (nhp, fhp) = (w.body.enemies.hp[ni], w.body.enemies.hp[fi]);
        press(&mut w, BTN_BOMB);
        let f = w
            .body
            .fields
            .iter_alive()
            .find(|&i| w.body.fields.flags[i] & crate::field::FIELD_DAMAGE != 0)
            .expect("应铺了伤害 field");
        assert_eq!((w.body.fields.x[f], w.body.fields.y[f]), (Fx::ZERO, Fx::from_int(200)));
        press(&mut w, 0);
        assert!(w.body.enemies.hp[ni] < nhp, "圈内敌必须掉血");
        assert_eq!(w.body.enemies.hp[fi], fhp, "圈外敌不得掉血");
    }

    /// 持续消弹：起爆后第 60 帧新来的弹也被消掉（`life = 1` 的错实现会红）。
    #[test]
    fn classic_bomb_clear_field_keeps_clearing_for_its_whole_duration() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        press(&mut w, BTN_BOMB);
        for _ in 0..59 {
            press(&mut w, 0);
        }
        bullet_at(&mut w, 0, 200);
        press(&mut w, 0);
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "整段期间新弹也该被消掉");
    }

    /// 沿检测三件：按住不连环 / 结束后真沿再发 / 进行中再按 no-op 不扣不刷新。
    #[test]
    fn classic_holding_the_bomb_key_does_not_chain_bomb() {
        let mut w = classic_world();
        w.body.players[0].bombs = 3;
        let frames = classic_bomb().frames;
        for _ in 0..=(2 * frames + 4) {
            press(&mut w, BTN_BOMB);
        }
        assert_eq!(w.body.players[0].bombs, 2, "全程按住只该起爆一次");
    }

    #[test]
    fn classic_genuine_second_bomb_after_release_fires_again() {
        let mut w = classic_world();
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        for _ in 0..classic_bomb().frames {
            press(&mut w, 0);
        }
        assert_eq!(w.body.players[0].bomb_timer, 0, "本段应已结束");
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "新的真沿必须照常起爆");
    }

    #[test]
    fn classic_pressing_bomb_while_one_is_active_is_a_free_noop() {
        let mut w = classic_world();
        w.body.players[0].bombs = 2;
        press(&mut w, BTN_BOMB);
        press(&mut w, 0);
        let left = w.body.players[0].bomb_timer;
        assert!(left > 0);
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 1, "效果中再按不得扣");
        assert!(w.body.players[0].bomb_timer < left, "不得刷新计时");
    }

    /// 计时恰好 `frames` 帧归零：触发帧不被自己减，第 frames−1 帧仍 >0，第 frames 帧 ==0。
    #[test]
    fn classic_bomb_timer_runs_out_exactly_after_frames() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        let frames = classic_bomb().frames;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_timer, frames, "触发当帧不被自己减掉");
        for k in 1..frames {
            press(&mut w, 0);
            assert_ne!(w.body.players[0].bomb_timer, 0, "第 {k} 帧仍应在效果中");
        }
        press(&mut w, 0);
        assert_eq!(w.body.players[0].bomb_timer, 0);
    }

    /// 门禁：无库存无效；ECL 演出冻结（A 组）发不出、不扣。
    #[test]
    fn classic_bomb_gates_stock_and_actor_freeze() {
        let mut w = classic_world();
        w.body.players[0].bombs = 0;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_timer, 0);
        assert_eq!(w.body.fields.iter_alive().count(), 0, "不得铺任何作用区");
        let mut w = classic_world();
        w.body.players[0].bombs = 2;
        w.body.freeze_left = [0, 10];
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.players[0].bomb_timer, 0, "被定住期间不得发动");
        assert_eq!(w.body.players[0].bombs, 2);
    }

    /// 空 `fields` 合法：不铺区，但扣库存、进效果段、给无敌（触发帧不被自减）。
    #[test]
    fn classic_bomb_with_no_fields_still_grants_invulnerability() {
        let mut t = crate::tables::build_tables_v0();
        let crate::tables::Kit::Classic(b) = &mut t.characters[1].kit else { unreachable!() };
        b.fields = Box::new([]);
        let invuln = b.invuln;
        let mut w = crate::step::World::new_with_tables(1, &t);
        w.body.players[0] = crate::player::PlayerState::spawn(1, &t.characters[1]);
        w.body.players[0].bombs = 1;
        let mut input = InputFrame::empty(w.frame());
        input.actions[0].buttons = BTN_BOMB;
        crate::step::step(&mut w, &t, &crate::ecl::image::EclImage::empty(), &input);
        assert_eq!(w.body.players[0].bombs, 0);
        assert_ne!(w.body.players[0].bomb_timer, 0);
        assert_eq!(w.body.fields.iter_alive().count(), 0);
        assert_eq!(w.body.players[0].invuln, invuln);
    }

    /// 起爆当帧全屏吸道具 + 当帧符卡失格。
    #[test]
    fn classic_bomb_attracts_items_and_voids_the_spell_capture() {
        let mut w = classic_world();
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
        let h = w.body.drop_item(
            crate::math::Fx::ZERO,
            crate::math::Fx::from_int(60),
            crate::items::ITEM_POWER,
            &crate::tables::TABLES_V0,
        );
        let ii = w.body.items.get(h).unwrap();
        w.body.players[0].bombs = 1;
        press(&mut w, BTN_BOMB);
        assert_eq!(w.body.items.magnet_to[ii], 0, "起爆当帧应全场上锁到自机 0");
        assert_eq!(w.body.spells[0].capture_ok, 0, "起爆后本卡不予收卡");
    }

    /// 跨套件判别（X 键）：同一按键在机体 0 = 停止、机体 1 = bomb。防分派接反/写死一边。
    #[test]
    fn x_key_dispatches_by_kit() {
        let mut chronos = crate::step::World::new(1);
        chronos.body.players[0].bombs = 1;
        press(&mut chronos, BTN_BOMB);
        assert_eq!(chronos.body.freeze_left[0], crate::player::TIMESTOP_FRAMES);
        assert_eq!(chronos.body.players[0].bomb_timer, 0);
        assert_eq!(chronos.body.fields.iter_alive().count(), 0);

        let mut classic = classic_world();
        classic.body.players[0].bombs = 1;
        press(&mut classic, BTN_BOMB);
        assert_eq!(classic.body.freeze_left[0], 0, "Classic 不得停止");
        assert_eq!(classic.body.players[0].bomb_timer, classic_bomb().frames);
        assert_eq!(classic.body.fields.iter_alive().count(), 2);
    }

    /// `bomb_timer` 进校验和（P6）。
    #[test]
    fn bomb_timer_enters_the_checksum() {
        let mut w = classic_world();
        let c0 = w.checksum();
        w.body.players[0].bomb_timer = 1;
        assert_ne!(w.checksum(), c0);
    }

    /// 容量闸：rank-3 峰值 814 弹下起 bomb 跑满整段，道具池不溢出。若变红**不要现场调池 cap**，记录实测留给人裁定。
    #[test]
    fn classic_bomb_at_rank3_peak_bullet_count_does_not_overflow_item_pool() {
        use crate::world::POOL_ITEM;
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        for _ in 0..814 {
            bullet_at(&mut w, 0, 224);
        }
        press(&mut w, BTN_BOMB);
        for _ in 1..classic_bomb().frames {
            press(&mut w, 0);
        }
        assert_eq!(w.body.diag.pool_full[POOL_ITEM], 0);
    }
```

`mod tests` 顶部已有 `use crate::input::{BTN_BOMB, BTN_JUMP, InputFrame};`、`use crate::player::{JUMP_FRAMES, LIFE_JUMPING, REWIND_INVULN};`、`use crate::world::test_support::{bullet_at, step_t};`；`press` 定义在停止段（模块内顺序无关）。`w.checksum()` / `w.frame_events()` 是 `World` 的方法（同文件 `jump_cd_enters_the_checksum`、`deathwindow_expiry_continues_in_place_and_requests_rewind` 同款）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib world::player::tests::classic 2>&1 | tail -20`
Expected: 编译错误 `no field bomb_timer`。

- [ ] **Step 3: 实现**

1. `player.rs` `PlayerState`：在 `jump_cd` 之后加

```rust
    /// 经典 bomb 剩余帧（经典机体刀）。`Kit::Classic` 的 `try_bomb` 写 `BombCfg.frames`，C 组逐帧减
    /// （场景冻结不走）；`!= 0` 即「bomb 进行中」，是二次起爆门禁的单一真相源。Chronos 机体恒 0。
    pub bomb_timer: u16,
```

   `spawn` 加 `bomb_timer: 0,`。`RESPAWN_INVULN` 注释改为「续关 + Classic 场底重生的无敌帧（2 秒 @60Hz）」。
2. `world/player.rs` 头部 `use crate::tables::WorldTables;` 改 `use crate::tables::{BombCfg, BombOrigin, Kit, WorldTables};`。
3. C 组：`jump_cd` 递减之后加

```rust
                // 经典 bomb 计时（经典机体刀）：C 组、任何 life_state 下都数；触发帧的写入在 A 组，
                // 故不被自己减掉。
                if self.players[i].bomb_timer > 0 {
                    self.players[i].bomb_timer -= 1;
                }
```

4. A 组：把

```rust
            self.try_jump(i);
            if self.players[i].life_state == LIFE_JUMPING {
                continue;
            }
            self.try_stop(i);
```

   改为

```rust
            // 规则套件分派（经典机体刀）：X/C 键行为按机体数据走，穷尽 match。
            match &tables.characters[self.players[i].character_id as usize].kit {
                Kit::Chronos => {
                    self.try_jump(i);
                    if self.players[i].life_state == LIFE_JUMPING {
                        continue;
                    }
                    self.try_stop(i);
                }
                Kit::Classic(bomb) => self.try_bomb(i, bomb),
            }
```

5. `try_stop` 之后加

```rust
    /// 经典 bomb（`Kit::Classic`，经典机体刀 spec §3.1）。门禁：上升沿 + 库存 > 0 + 未在 bomb 中
    /// + ALIVE 或决死窗口（deathbomb；命没扣，无退款）。效果顺序固定：扣库存 → 计时 → 救窗口
    /// → 无敌 → 按声明序铺 field（I4）→ 吸道具（此时已 ALIVE）→ 触发点失格（与 `try_stop` 同口径）。
    /// `cfg` 借自 `tables`（与 `self` 不同对象）；按索引取 field 免得迭代器横跨 `create_field`。
    fn try_bomb(&mut self, i: usize, cfg: &BombCfg) {
        if !self.pressed_edge(i, crate::input::BTN_BOMB)
            || self.players[i].bombs == 0
            || self.players[i].bomb_timer != 0
            || !matches!(self.players[i].life_state, LIFE_ALIVE | LIFE_DEATHWINDOW)
        {
            return;
        }
        let p = &mut self.players[i];
        p.bombs -= 1;
        p.bomb_timer = cfg.frames;
        if p.life_state == LIFE_DEATHWINDOW {
            p.life_state = LIFE_ALIVE;
            p.state_timer = 0;
        }
        p.invuln = cfg.invuln;
        let (px, py) = (p.x, p.y);
        for k in 0..cfg.fields.len() {
            let f = cfg.fields[k];
            let (x, y) = match f.origin {
                BombOrigin::FieldCenter => (Fx::ZERO, Fx::from_int(super::FIELD_HEIGHT / 2)),
                BombOrigin::PlayerAtCast => (px, py),
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
        self.void_spell_captures();
    }
```

6. 更新 `step.rs` 的 `world_size_sentinel_guards_copy_into_field_list`：跑测试看实测尺寸；若响了改数值并加一段注释「2026-09-15（经典机体刀 T2）：`PlayerState.bomb_timer: u16` ×2 …实测 …」；若没响（padding 吸收，D20）也补一句记账。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p stg-core --lib world::player 2>&1 | tail -5 && cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked" | head -20`
Expected: 全过。机体 0 的停止 / 跳躍测试原样绿。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/player.rs crates/stg-core/src/world/player.rs crates/stg-core/src/step.rs
git commit -m "feat(core): 经典机体刀 T2——PlayerState.bomb_timer + try_bomb + X/C 键按 Kit 分派"
```

---

### Task 3: 经典死亡——场底重生 + C 键判别 + timeline

**Files:**
- Modify: `crates/stg-core/src/world/player.rs`（`commit_death` 分派）
- Modify: `crates/stg-core/src/timeline.rs`（测试）

**Interfaces:**
- Consumes: Task 2 的 `classic_world()` 测试辅助、A 组分派。
- Produces: `fn commit_death(&mut self, i: usize, tables: &WorldTables)`。

- [ ] **Step 1: 写失败测试**（`world/player.rs` 经典段末尾）

```rust
    /// Classic 死亡：窗口耗尽 → 场底 (0,384)、ALIVE、RESPAWN_INVULN、残机 −1、偏差 +1、**无遡行请求**。
    #[test]
    fn classic_death_respawns_at_field_bottom_without_rewind_request() {
        use crate::math::Fx;
        let mut w = classic_world();
        w.body.players[0].x = Fx::from_int(-100);
        w.body.players[0].y = Fx::from_int(150);
        let lives0 = w.body.players[0].lives;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0);
        let p = w.body.players[0];
        assert_eq!((p.x, p.y), (Fx::ZERO, Fx::from_int(384)), "场底中心");
        assert_eq!(p.life_state, crate::player::LIFE_ALIVE);
        assert_eq!(p.invuln, crate::player::RESPAWN_INVULN);
        assert_eq!((p.lives, p.deaths), (lives0 - 1, 1));
        assert!(
            w.frame_events().iter().any(|e| e.kind == crate::events::EVT_PLAYER_DIED),
            "照发 PlayerDied"
        );
        assert!(
            !w.frame_events().iter().any(|e| e.kind == crate::events::EVT_REWIND_REQUESTED),
            "Classic 不得发遡行请求"
        );
    }

    #[test]
    fn classic_last_life_death_enters_gameover() {
        let mut w = classic_world();
        w.body.players[0].lives = 1;
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
        w.body.players[0].state_timer = 1;
        press(&mut w, 0);
        assert_eq!(w.body.players[0].life_state, crate::player::LIFE_GAMEOVER);
    }

    /// 跨套件判别（死亡）：机体 0 原地 + REWIND_INVULN + 请求；机体 1 场底 + RESPAWN_INVULN + 无请求。
    #[test]
    fn death_dispatches_by_kit() {
        use crate::math::Fx;
        let die = |mut w: Box<crate::step::World>| {
            w.body.players[0].x = Fx::from_int(-100);
            w.body.players[0].y = Fx::from_int(150);
            w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW;
            w.body.players[0].state_timer = 1;
            press(&mut w, 0);
            let asked = w
                .frame_events()
                .iter()
                .any(|e| e.kind == crate::events::EVT_REWIND_REQUESTED);
            (w.body.players[0].x, w.body.players[0].y, w.body.players[0].invuln, asked)
        };
        assert_eq!(
            die(crate::step::World::new(1)),
            (Fx::from_int(-100), Fx::from_int(150), REWIND_INVULN, true)
        );
        assert_eq!(
            die(classic_world()),
            (Fx::ZERO, Fx::from_int(384), crate::player::RESPAWN_INVULN, false)
        );
    }

    /// Classic 按 C：永不进 JUMPING、`jump_cd` 不动；同帧 X 照常 bomb（C 不吞 X）。
    #[test]
    fn classic_c_key_is_a_noop() {
        let mut w = classic_world();
        w.body.players[0].bombs = 1;
        for _ in 0..(JUMP_FRAMES + 2) {
            press(&mut w, BTN_JUMP);
            assert_ne!(w.body.players[0].life_state, LIFE_JUMPING);
        }
        assert_eq!(w.body.players[0].jump_cd, 0);
        press(&mut w, 0);
        press(&mut w, BTN_JUMP | BTN_BOMB);
        assert_eq!(w.body.players[0].bombs, 0, "C+X 同帧 X 照常起爆");
    }
```

`timeline.rs` `mod tests` 末尾：

```rust
    /// Classic 机体挂 Timeline：被弹致死不遡行，帧号连续，落在场底（经典机体刀 spec §3.3）。
    #[test]
    fn classic_death_under_timeline_does_not_rewind() {
        let mut w = World::new(1);
        w.body.players[0] =
            crate::player::PlayerState::spawn(1, &crate::tables::TABLES_V0.characters[1]);
        // 先挪离出生点：出生点就是 (0,384)，不挪的话「落在场底」断言对重生是瞎的。
        w.body.players[0].x = crate::math::Fx::from_int(-100);
        w.body.players[0].y = crate::math::Fx::from_int(200);
        let mut t = Timeline::from_world(w, EclImage::empty(), Boot::Snapshot { world_checksum: 0 });
        plant_hit(&mut t);
        for _ in 0..=(DEATHBOMB_WINDOW as u32 + 2) {
            let before = t.frame();
            assert_eq!(t.advance(&InputFrame::empty(0)).rewound, None, "Classic 不得遡行");
            assert_eq!(t.frame(), before + 1, "帧号连续");
        }
        assert_eq!(t.world.body.players[0].deaths, 1, "确实死过一次");
        assert_eq!(t.world.body.players[0].y, crate::math::Fx::from_int(384));
    }
```

（`plant_hit` 放的弹停在自机原位；重生瞬移后自机离开，不会连环死。若 `Boot::Snapshot` 字段名不同，以 `timeline.rs` 定义为准。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core --lib classic_death 2>&1 | tail -20`
Expected: `classic_death_respawns…` FAIL（位置仍是 (-100,150)、有遡行请求）；timeline 测试 FAIL（`rewound` 为 Some）。

- [ ] **Step 3: 实现**

`commit_death` 改签名并分派；调用点 `self.commit_death(i);` → `self.commit_death(i, tables);`：

```rust
    /// 决死窗口耗尽。共用：扣残机 + 偏差值 + `EVT_PLAYER_DIED`；残机耗尽 → GAMEOVER。
    /// 之后按规则套件分派（经典机体刀）：
    /// - `Chronos`（玩法刀 spec §4.2，死亡即遡行）：**原地**回 ALIVE、`REWIND_INVULN`，发
    ///   `EVT_REWIND_REQUESTED`。有 timeline 的宿主据此恢复被弹前的快照，代价在 `rewind_landed`
    ///   从快照重算；无 timeline 的宿主到此为止 = 原地继续。
    /// - `Classic`（东方原作语义）：瞬移场底 `(0, 384)`、ALIVE、`RESPAWN_INVULN`，**不发**请求——
    ///   挂 timeline 也不会遡行。不新增生命态（旧 `LIFE_RESPAWNING` 期间 A 组本就照跑，与 ALIVE+无敌同）。
    fn commit_death(&mut self, i: usize, tables: &WorldTables) {
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
        let cid = p.character_id as usize;
        // 各臂内重新取 `self.players[i]`：Chronos 臂要调 `self.push_event`，不能跨调用持有 `p`。
        match &tables.characters[cid].kit {
            Kit::Chronos => {
                let p = &mut self.players[i];
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
            Kit::Classic(_) => {
                let p = &mut self.players[i];
                p.x = Fx::ZERO;
                p.y = Fx::from_int(384);
                p.invuln = p.invuln.max(crate::player::RESPAWN_INVULN);
            }
        }
    }
```

模块头注释「→ 原地继续 + 遡行请求 / GAMEOVER」改为「→ 按 Kit：原地继续 + 遡行请求 / 场底重生 / GAMEOVER」。

- [ ] **Step 4: 跑测试**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked" | head -20`
Expected: 全过。

- [ ] **Step 5: Commit**

```bash
git add crates/stg-core/src/world/player.rs crates/stg-core/src/timeline.rs
git commit -m "feat(core): 经典机体刀 T3——commit_death 按 Kit 分派（Classic 场底重生不遡行）+ C 键/死亡跨套件判别 + timeline 腿"
```

---

### Task 4: ENGINE_VER 22 + 元数据 + 文档 + 全闸门收口

**Files:**
- Modify: `crates/stg-core/src/lib.rs`、`crates/stg-core/src/step.rs`（`engine_ver_anchored`）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（`add_bombs` doc）→ `gen-ecl-meta` 产物（`docs/ecl-lang/7-reference.md`、`editors/vscode/stg-ecl/ecl-meta.json`）
- Modify: `docs/ecl-ops.md`、`docs/ecl-lang/6-spell-and-stage.md`、`stg-world-design.md`、`docs/architecture.md`、`docs/follow-ups.md`、`PROGRESS.md`、`CLAUDE.md`、spec 状态行

- [ ] **Step 1: 改锚定测试为 22（先红）**

`step.rs` `engine_ver_anchored`：`21` → `22`，消息首段改为

```
"bump 必须是有意识决定(评审 + 改本测试)——21→22：经典机体刀(2026-09-15)。\
 PlayerState 加 bomb_timer u16(存档 wire format 变);WorldTables 加 CharacterCfg.kit 规则套件、\
 characters 变长(TABLE_VERSION 6,content_hash 变);机体 1 = Kit::Classic(bomb/场底重生/无跳躍),机体 0 行为不变。\
 ——前一次 20→21：boss 换段与敌人钩子刀(2026-09-14)。…"
```

（原 20→21 段落保留接在后面。）

Run: `cargo test -p stg-core --lib engine_ver_anchored 2>&1 | tail -3` → Expected: FAIL（21 ≠ 22）。

- [ ] **Step 2: bump**

`lib.rs`：`ENGINE_VER` 文档史末尾追加

```
/// **21 → 22**（经典机体刀，2026-09-15）：**布局 + 表格式两重**。① `PlayerState.bomb_timer: u16`
/// 进校验和与存档；② `WorldTables` 加 `CharacterCfg.kit: Kit`（`Chronos` / `Classic(BombCfg)`）、
/// `characters` 由定长 1 改变长（`TABLE_VERSION` 6）⇒ 表 `content_hash` 变。行为：机体 0 零改动；
/// 机体 1（RL 训练机体）X = bomb、C 无、死亡场底重生不遡行。**金向量预期改变**（表哈希进校验和），实测为准。
```

`pub const ENGINE_VER: u32 = 22;`。

Run: `cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked" | head` → 全过。

- [ ] **Step 3: 元数据文案**

`builtins.rs` `add_bombs` 的 `doc` 改为
`"增减 X 键库存(時環晷机体=停止,经典机体=bomb):delta 允许负,双边钳 [0,STOP_STOCK_MAX=5] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_"`。

Run: `cargo run -p stg-harness -- gen-ecl-meta && git diff --stat`
Expected: `7-reference.md` 与 `ecl-meta.json` 各一行变化。

- [ ] **Step 4: 手写文档**

- `docs/ecl-ops.md` 511 行：「写 `bombs`（停止库存）」→「写 `bombs`（X 键库存：Chronos 停止 / Classic bomb）」。
- `docs/ecl-lang/6-spell-and-stage.md` ~412 行附近补一句：「机体 1（经典训练机体）的 X 是经典 bomb，库存同一份。」；~473 表格 `add_bombs` 的钳位 `[0, 255]` 与 `syscall.rs` 实际 `[0, 5]` 不符，顺手订正为 `[0, STOP_STOCK_MAX=5]`。
- `stg-world-design.md` D6：生死状态机表下加一段

```
> **规则套件 `Kit`（经典机体刀 2026-09-15）**：`CharacterCfg.kit` 按机体分派三处——A 组 X 键
> （`Chronos` → `try_stop` / `Classic(BombCfg)` → `try_bomb`）、A 组 C 键（Chronos → `try_jump` /
> Classic 无）、`commit_death`（Chronos 原地 + `EVT_REWIND_REQUESTED` / Classic 场底 `(0,384)` +
> `RESPAWN_INVULN`）。bomb 状态单一字段 `bomb_timer: u16`（C 组计时）。机体 1 = RL 训练机体。
```

  并把表格 `~~bomb 状态机~~` 行尾追加「；经典机体刀恢复 `bomb_timer: u16`（仅 Classic 用）」。
- `docs/architecture.md` M5 行「接缝」列追加「；训练机体 = 机体 1（`Kit::Classic`，经典机体刀）」。
- `docs/follow-ups.md` 在 D21 之后加：

```
### D22. 经典机体刀的非目标（spec §7 记档，2026-09-15）

机体 1（`Kit::Classic`）只保证核内行为。以下未做，**触发点**各自写明：
1. Godot 壳 / 桥 HUD 不适配机体 1（冷却条、停止文案、bomb 演出）——壳里选机体 1 能跑但表现未校。触发点：想在 Godot 里人肉玩经典机体。
2. `BombField` 只有圆；激光形 bomb 须改碰撞矩阵行 6/7。触发点：迁移目标作的 bomb 形状影响训练。
3. 机体 1 复用机体 0 火力 / 移速，不对齐任何原作机体。触发点：迁移验证发现差异显著。
4. 场底重生无入场动画 / 不可操作帧（原作有）。触发点：同上。
5. `STOP_STOCK_MAX` 两套件共用（原作 bomb 上限常为 8）。触发点：同上。
```

- `CLAUDE.md` 仓库结构段 `src/{boss,tables}.rs` 描述里「shottype+道具+角色参数+appearance」→「shottype+道具+角色参数+规则套件 Kit+appearance」。
- spec 状态行改「**已落地（2026-09-15）**」，若实施有偏差在文末加 §8 偏差表。

- [ ] **Step 5: 全闸门**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep -E "^test result|FAILED|panicked"
cargo run -p stg-harness -- verify-tables
cargo run --release -p stg-harness -- storm
cargo run -p stg-harness -- golden --out /tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/eef4a324-1071-454a-99d8-cc232ee064b4/scratchpad/golden_new.txt && md5sum /tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/eef4a324-1071-454a-99d8-cc232ee064b4/scratchpad/golden_new.txt
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```

Expected: 全绿；两冒烟 `SMOKE OK`；记下 golden md5。**金向量行为不变的证据**：`git worktree add` 一次性副本跑 base（7639cbc）golden，与新流逐帧对比——两者差异应只来自 `tables_hash`（每帧都变、但机体 0 的位置/弹数等可从 `run` 输出核对）。做法：两个 worktree 各跑 `cargo run -p stg-harness -- run godot/ecl/game --frames 600`，比对计数/峰值/末帧输出**完全相同**。

- [ ] **Step 6: PROGRESS**

`PROGRESS.md`「现在」段重写为经典机体刀落地 + 下一步「`stg-py` env 刀（spec 另起：观测编码按 stg-agent-proto HELLO 字段名 / reset = copy_into / 批量 env / 机体 1 训练）」与既有「第 1 关内容刀」并列；里程碑史表首插一行（ENGINE_VER 22、TABLE_VERSION 6、测试计数、金向量 md5、闸门）。

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: ENGINE_VER 22（经典机体刀）+ add_bombs 元数据 + 文档收口（world-design D6/architecture/ecl 手册/ecl-ops/follow-ups D22/PROGRESS/CLAUDE.md）"
```
