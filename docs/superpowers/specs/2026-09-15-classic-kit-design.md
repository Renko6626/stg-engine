# 经典机体刀 —— 按机体数据分派的规则套件 `Kit`（设计，2026-09-15）

> 状态：**已落地（2026-09-15）**。
> 来源：RL 路线 A（在 stg-engine 上 headless 高速训练 → 迁移到 th06nc / TH18，契约见
> `stg-agent-proto` 与 `renkolab/docs/superpowers/specs/2026-09-10-ai-agent-bridge-design.md` §6）。
> 训练用机体要东方原作语义（bomb = 范围消弹 + 伤害、场底重生、无跳躍），游戏本体（東方時環晷）
> 的停止 / 跳躍 / 死亡即遡行**逐字节不变**。
> 本刀只做核内机体数据；`stg-py` env（观测编码 / reset / 批量）是下一刀，另起 spec。
> 前置：玩法刀（2026-09-14，633c3f1 删了旧 `BombCfg` 族、27fa76b 退役场底重生）。

## 1. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 要不要 fork RL 特化 core | **不 fork**。RL 是断层线以上的消费者；版本漂移靠钉 commit + HELLO `stg-engine@<VER>` 管 |
| ② | 经典 bomb 住哪 | **按机体数据分派**（`CharacterCfg.kit`），不加开机 ruleset 开关、不改游戏本体 |
| ③ | 经典机体的死亡 / 跳躍 | **场底重生也恢复**；C 键 no-op |
| ④ | 表达形式 | **一个枚举打包三件事**（方案 ①），不做三个正交字段（YAGNI）、不按 `character_id` 硬分支（规则须进表哈希） |
| ⑤ | 细节 | 不新增生命态（重生 = 瞬移 + ALIVE + 无敌）；机体 1 复用机体 0 的移速/判定/火力；bomb 数值取旧 v0 表；Godot 壳 HUD 不适配机体 1 |

## 2. 数据层（`tables.rs`）

### 2.1 类型

```rust
pub struct CharacterCfg {
    pub high_speed: Fx, pub low_speed: Fx, pub inv_sqrt2: Fx,
    pub hit_radius: Fx, pub graze_radius: Fx,
    pub shot: ShotTypeCfg,
    pub kit: Kit,                       // 新
}

/// 机体规则套件：X 键 / C 键 / 死亡三处行为打包。三个分派点一律穷尽 `match`（D18 手法）。
pub enum Kit {
    /// 東方時環晷：X = 停止，C = 跳躍，死亡 = 原地继续 + `EVT_REWIND_REQUESTED`。
    Chronos,
    /// 东方原作语义：X = bomb，C = 无，死亡 = 场底重生。
    Classic(BombCfg),
}

pub struct BombCfg { pub frames: u16, pub invuln: u16, pub attract_items: bool, pub fields: Box<[BombField]> }
pub struct BombField { pub origin: BombOrigin, pub radius: Fx, pub flags: u8, pub dmg_per_frame: u16, pub life: u16 }
pub enum BombOrigin { FieldCenter, PlayerAtCast }
```

`BombCfg` / `BombField` / `BombOrigin` 从 `633c3f1^:crates/stg-core/src/tables.rs` 原样捞回（含文档注释里
「数据变不出新形状」那段）。

### 2.2 角色表形态

`WorldTables.characters: [CharacterCfg; 1]` → **`Box<[CharacterCfg]>`**。`validate` 增加 `!characters.is_empty()`；
每个 `Classic` 的 `fields` 逐条 `radius_in_range`。`PlayerState.character_id` 只经 `new_game_at`（已校验
`< len`）与 `World::new_with_tables`（恒 0）写入，索引不新增越界面。

### 2.3 规范字节 v6

`TABLE_VERSION` 5 → **6**。角色段改为：`count: u32`（≥1，0 → `ArityMismatch{field:"characters"}`），
每个角色在既有字段（移速/半径/shot）之后追加：

```
kit_tag: u8            0 = Chronos, 1 = Classic；其它 → BadDiscriminant{field:"kit"}
[Classic 时]
  frames: u16, invuln: u16, attract_items: u8 (0/1；其它 → BadDiscriminant{field:"attract_items"})
  n_fields: u32
  每条：origin: u8 (0 FieldCenter / 1 PlayerAtCast；其它 → BadDiscriminant{field:"bomb_origin"})
        radius: i32, flags: u8, dmg_per_frame: u16, life: u16
```

`tables_v0.bin` 由 `bake-tables` 重烘并 commit，`verify-tables` 逐位对拍。

### 2.4 v0 内容

- **机体 0**：现值不动，`kit: Chronos`。
- **机体 1**：移速/判定/擦弹/shot 与机体 0 逐字段相同，`kit: Classic(BombCfg { frames: 120, invuln: 120,
  attract_items: true, fields: [全屏消弹圆(FieldCenter, FIELD_RADIUS_FULLSCREEN, CLEAR_BULLETS, 0, 120),
  起爆点伤害圆(PlayerAtCast, 120, DAMAGE, 4, 120)] })`——旧 v0 数值原样。

## 3. 行为层（`world/player.rs`）

三个分派点，都读 `tables.characters[character_id].kit`，穷尽 `match`：

| 分派点 | `Chronos`（现状，逐字节不变） | `Classic` |
|---|---|---|
| A 组 X 键 `try_x` | `try_stop` | `try_bomb` |
| A 组 C 键 | `try_jump` | 不调用（`jump_cd` 恒 0，永不进 `LIFE_JUMPING`） |
| C 组 `commit_death` | 原地 ALIVE + `REWIND_INVULN` + `EVT_REWIND_REQUESTED` | 瞬移 `(0, 384)` + ALIVE + `RESPAWN_INVULN`(120)，**不发**遡行请求 |

两种套件共用的部分不动：决死窗口 8 帧、残机/偏差值/`EVT_PLAYER_DIED`、`lives == 0 → GAMEOVER`、
死亡帧 A 组跳过、`try_continue`（原地复活 + `RESPAWN_INVULN`）。

### 3.1 `try_bomb`

门禁：`pressed_edge(BTN_BOMB)` ∧ `bombs > 0` ∧ `bomb_timer == 0` ∧ `life_state ∈ {ALIVE, DEATHWINDOW}`。效果（顺序固定）：

1. `bombs -= 1`；`bomb_timer = cfg.frames`。
2. `DEATHWINDOW → ALIVE`、`state_timer = 0`（deathbomb；命没扣，无退款）。
3. `invuln = cfg.invuln`。
4. 按声明序铺 `fields`（`FieldCenter` = `(0, FIELD_HEIGHT/2)`，`PlayerAtCast` = 起爆当帧自机位；`owner = i`）。
5. `attract_items` → `attract_all_items(i)`（此时已 ALIVE，满足该 API 门禁）。
6. `void_spell_captures()`——与 `try_stop` 同口径在**触发点**失格（不恢复旧的 `bomb_phase` 轮询支）。

ECL 演出冻结（A 组冻）期间按不出——A 组门禁自带。场景冻结（`freeze_left[0]`）在 Classic 下没有生产者
（Classic 没有停止），但 ECL `time_stop_player` 仍可能冻 A 组，行为同上。

### 3.2 `bomb_timer`（新字段，C 组计时）

`PlayerState.bomb_timer: u16`：触发写 `cfg.frames`；C 组（`!scene`）任何 life_state 下 `> 0` 则减一。
**不恢复 `bomb_phase`**——`bomb_timer != 0` 即「bomb 进行中」，单一真相源。触发帧 C 组先于 A 组跑，
故不被自己减掉。Chronos 机体恒 0。

### 3.3 与 timeline 的关系

Classic 永不发 `EVT_REWIND_REQUESTED` ⇒ 挂 `Timeline` 的宿主（Godot 壳、harness replay）不会遡行，
`rewind_landed` 不被调用；観測影子喂 `BTN_JUMP` 对 Classic 是 no-op，影子 = 正常前推。无需改 `timeline.rs`。

## 4. 布局与版本

- `PlayerState` + `bomb_timer: u16` ⇒ 存档 wire format 变；尺寸哨兵可能被 padding 吃掉（D20），实测记账。
- **`ENGINE_VER` 21 → 22**：布局 + 表格式（`TABLE_VERSION` 6）两重。
- 金向量：`tables_hash` 进校验和 ⇒ **md5 预期改变**；机体 0 行为零改动（rainbow 与风铃卡都不用机体 1），实测为准。
- 输入词表、syscall 号表、碰撞矩阵、step 顺序、事件号：**全不动**。`STOP_STOCK_MAX` 两种套件共用
  （Classic 的 bomb 库存上限也是 5，名字不改）。

## 5. 测试（判别式）

tables：
- v6 往返：Classic 机体（多条 field、两种 origin、非零伤害）序列化再解析逐字段相等。
- 坏 `kit_tag` / 坏 `bomb_origin` / 坏 `attract_items` → 各自 `BadDiscriminant`；`characters` 计数 0 → `ArityMismatch`。
- `TABLES_V0` 机体 1 与机体 0 除 `kit` 外逐字段相等；机体 1 bomb 数值钉旧 v0。

behavior（捞回并改写旧 bomb 测试组，一律用机体 1 世界；另加跨套件判别腿）：
- deathbomb 窗口内救且不扣命 / 窗口耗尽后按 X 救不回、不扣 bomb（成对）。
- 伤害圆圈内掉血圈外不掉（几何可分辨）；伤害圆圆心 = 起爆点 ≠ 场心。
- 全屏消弹持续整段（第 60 帧新弹也被消）。
- 按住不连环 / 松开等结束后真沿再发 / 进行中再按 no-op 不扣不刷新（沿检测三件）。
- `bomb_timer` 恰好 `frames` 帧归零；ECL 演出冻结下发不出、不扣；无库存无效；空 `fields` 合法仍给无敌。
- 起爆全屏吸道具；起爆当帧符卡失格。
- Classic 死亡：窗口耗尽 → 位置 `(0,384)`、ALIVE、`invuln == RESPAWN_INVULN`、残机 −1、偏差 +1、
  **帧事件无 `EVT_REWIND_REQUESTED`**；末命 → GAMEOVER。
- Classic 按 C：永不进 JUMPING、`jump_cd` 不变。
- **跨套件判别**：同一输入序列（按 X / 按 C / 被弹致死）在机体 0 与机体 1 上走出不同结果——
  机体 0 `freeze_left[0] == TIMESTOP_FRAMES`、发遡行请求、原地；机体 1 铺 field、`bomb_timer == 120`、场底。
  防「分派接反」「分派写死成一边」。
- `bomb_timer` 进校验和（改它 checksum 变）。
- 容量闸：机体 1 在 rank-3 峰值 814 弹下起 bomb 跑满 120 帧，道具池不溢出（旧 `bomb_at_rank3_peak…` 捞回）。
- Timeline：机体 1 开机的 `Timeline` 被弹致死后 `Advance.rewound == None`、帧号连续。

回归：机体 0 的停止 / 跳躍 / 遡行测试全部原样绿（证明 Chronos 逐字节不变）。

## 6. 文档同步

- `stg-world-design.md` 自机节：`Kit` 分派三点。
- `docs/architecture.md`：M5 接缝行补「训练机体 = 机体 1 `Kit::Classic`」。
- `docs/ecl-lang/7-reference.md` / `builtins.rs`：`add_bombs` 文案「停止库存（Classic 机体为 bomb 库存）」→ `gen-ecl-meta` 同步。
- `docs/follow-ups.md`：新记 D22（本刀非目标）。
- `PROGRESS.md`：史一行 + 「现在」段。
- `CLAUDE.md` 仓库结构段 `tables` 描述补 `Kit`。

## 7. 非目标（记 D22）

- Godot 壳 / 桥 HUD 对机体 1 的适配（冷却条、停止文案、bomb 表现）——RL headless 不看；桥 `new_game_at`
  的 `character` 参数已透传，壳里选机体 1 能跑但表现未校。
- 各作 bomb 差异（TH18 卡牌、th06 各机体 bomb 形状）——`BombField` 只有圆；要激光形状须改碰撞矩阵，另开刀。
- 自机火力 / 移速差异化——机体 1 复用机体 0。
- 场底重生的「入场动画 / 不可操作帧」——原作有，训练迁移若需要再议。
- `stg-py` env 本身（下一刀）。

## 8. 实施偏差

| # | 偏差 | 处置 / 现状 |
|---|---|---|
| ① | **机体 1 发弹分派**：计划 §3 只列了 X 键 / C 键 / `commit_death` 三处分派，漏写 `update_players` 里的**发弹角色分派**——不补则机体 1 选出来不开火。T1 审阅发现。 | T2 修：发弹分支改为 `0 \| 1 =>`，机体 1 复用机体 0 火力（`player.rs`）。 |
| ② | **冒烟与行为对拍**：brief Step 5 的两条 `run-smoke.sh`（`crates/stg-godot/smoke`、`godot/smoke`）与「`git worktree` 对拍 base 7639cbc」需在沙箱外写文件。 | 本任务只跑核内闸门（fmt/clippy/test/storm/verify-tables/golden）；两冒烟与机体 0 行为对拍由**收口复核**执行。 |
| ③ | **`void_spell_captures` 调用点口径**：计划/spec 正文只提 `try_stop` / `try_bomb` 两处，实际现有**三个**调用点。 | T3 审阅统一注释与文档为三处：`try_stop`（冻结期间 settle 不跑）、`try_bomb`（起爆不改生命态）、`rewind_landed`（快照带回了被弹前的资格）。 |
