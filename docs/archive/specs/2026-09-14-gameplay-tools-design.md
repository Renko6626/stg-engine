# 玩法刀 —— 停止合并 / 跳躍冷却 / 死亡即遡行 / 偏差值（设计，2026-09-14）

> 状态：**已落地（2026-09-14）**，实施偏差见 §10。玩法数字来源 `docs/gameplay-design.md` §1–§4（2026-09-12 十问）；
> 本文只定**引擎侧怎么实现**，§10 改动入口的 1–4、6、7 条。harness 探针（§10 第 5 条
> `probe-jump` / `replay --cut-gaps`）**不在本刀**，第 1 关内容落地后另开。
> 前置：时间机制内核刀（timeline，2026-09-07）、壳子刀（2026-09-11）。

## 1. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 旧 bomb | **整个删掉，名字不动**：`try_bomb`/`bomb_phase`/`bomb_timer`/`BombCfg` 族删；`BTN_BOMB`（位 5，X）改触发停止；`PlayerState.bombs`、`bomb_pieces`、ECL `add_bombs`、桥 `hud_player.bombs` 名字保留、语义 = 停止库存。`FieldPool` 保留（ECL 在用） |
| ② | 无 timeline 宿主的死亡 | **原地继续**：世界侧扣残机 + 偏差值 + 发遡行请求后原地回 ALIVE、`REWIND_INVULN` 无敌；`LIFE_RESPAWNING` 与场底重生整条删 |
| ③ | 范围 | 核 + 桥 + 壳一刀；harness 探针另开 |
| ④ | 落地残机下限 | `rewind_landed` 里 `lives = max(lives − 1, 1)`——**致死与否只在死的那一刻判定** |
| ⑤ | 観測按键 | 维持壳现行两下协议（按 C 开窗 → 窗内再按 C 跳），**窗口 60 → 180 tick（3 s）**；gameplay-design §2「按住」措辞随本刀改掉，按住式留给 V5 试玩再议 |

## 2. 停止合并（核）

### 2.1 输入词表

- `BTN_BOMB = 5, Edge` 名字不变，文档注释改为「停止」，消费者 `try_stop`。
- `BTN_TIMESTOP`（位 7）**删行，位号退役不复用**；`action_bit_values_frozen` 注明 7 退役。
- `vocab_hash_pinned` 换实测值。

### 2.2 触发 `try_stop`（A 组，取代 `try_time_stop` + `try_bomb`）

门禁：`pressed_edge(BTN_BOMB)` ∧ `bombs > 0` ∧ `freeze_left[0] == 0` ∧
`life_state ∈ {ALIVE, DEATHWINDOW}`。效果：

1. `bombs -= 1`；`freeze_left[0] = TIMESTOP_FRAMES`（180，常量不变）。
2. `DEATHWINDOW` → `ALIVE`、`state_timer = 0`（deathstop；残机本就未扣，无退款）。
3. 全部 active 符卡槽 `capture_ok = 0`——**必须在触发点写**：资格轮询住 settle，冻结期间
   settle 不跑，解冻后 `freeze_left[0]` 已归零，轮询永远看不到这次停止。

A 组调用序：`try_jump` → `try_stop` → `move_player` → 发弹（`try_rewind` 删）。ECL 演出冻结
（`freeze_left[1]`）期间 A 组不跑，演出中按不出停止——现状不变。

### 2.3 删除清单

- `world/player.rs`：`try_bomb`、`try_time_stop`、C 组 bomb 计时段。
- `player.rs`：`PlayerState.bomb_phase`/`bomb_timer`/`time_stops`；`Loadout.time_stops`
  （连带 `timeline.rs` 回放头少 1 字节 ⇒ `LOG_FILE_VER` 1 → 2）。
- `WorldBody::attract_all_items`：唯一生产调用方是 `try_bomb`，随之删（含 API 测试）；
  follow-ups C12⑤ 里「`attract_all_items` 未进 ECL」一句同步删。
- `tables.rs`：`BombCfg`/`BombField`/`BombOrigin`、`CharacterCfg.bomb`、其字节编解码与校验；
  表格式 version bump，`tables_v0.bin` 重烘（`bake-tables` → `verify-tables`）。
- `spell.rs`：资格轮询去掉 `bomb_phase != 0` 一支（只剩 `life_state != ALIVE`）。
- ECL：`SYS_ADD_TIME_STOPS`（513）**号退役不复用** + 内建 `add_time_stops`；
  `SYS_TIME_STOP_PLAYER`（560，演出定身 `freeze_left[1]`）**保留**。
- 随之删/改的测试：bomb 族、时停资源族、`try_continue` 对 `time_stops` 的断言。

### 2.4 触碰消弹（碰撞矩阵新行 8）

新行 `ROW_STOP_TOUCH = 8`：**冻结自机判定圆 × 冻住的敌弹**。仅在 `scene_frozen()`（玩家技能
冻结）时存在，三个相位函数各加一条冻结分支，**step 顺序不动**：

| 相位 | 未冻结 | 冻结（新） |
|---|---|---|
| 6 collide | 行 1–7 照旧 | 只收行 8：`life_state == ALIVE` 的自机（**不看 `invuln`**），`bullets.delay > 0` 跳过（与行 1/6 同口径），`d² ≤ (br + hit_radius)²` |
| 7 settle | 三趟照旧 | 只结算行 8：弹未带 `BULLET_CLEARED` → 置位 + 该自机 `score += STOP_TOUCH_SCORE`（10）。不转星星、不发事件、不跑符卡趟 |
| 9 cleanup | 照旧 | 只回收带 `BULLET_CLEARED` 的弹（场内必然，写 `vanished` `VANISH_CLEARED`；释放 xform 段）；寿命/越界/敌/道具/作用区一律不动 |

- 同帧多个自机碰到同一颗弹：按 hits 收集序首个入账（`CLEARED` 位挡后续），I4 升序。
- 冻结开始前一帧被标 `CLEARED` 的弹（旧推论「冻结中残留到解冻」）现在会在冻结首帧被回收——
  `cleanup.rs` 模块注释的那段推论改口径；`vanished_is_empty_while_scene_frozen` 改写为
  「冻结中只出现 `VANISH_CLEARED`」。
- 碰撞矩阵变更 = 过评审项，计入本刀 `ENGINE_VER` bump。

### 2.5 库存数字

- `items::PIECES_PER_BOMB` 5 → **4**（`PIECES_PER_LIFE` 5 不动）。
- 新常量 `player::STOP_STOCK_MAX = 5`，三个入口钳：
  - 碎片进位：`bomb_pieces >= 4` → 清零；`bombs < 5` 才加一（满了碎片照清、不加）。
  - `SYS_ADD_BOMBS`：钳 `[0, STOP_STOCK_MAX]`（原 `[0, 255]`）。
  - `new_game_at`：`loadout.bombs.min(STOP_STOCK_MAX)`。
- `Loadout::default().bombs` 3 → **2**；`try_continue` 回默认同源。
- 「保证掉落的整发停止」（gameplay-design §6.2）不新增道具类型：内容刀用 `add_bombs(1)` 或一次撒 4 碎片。

## 3. 跳躍冷却（核）

- `PlayerState.jump_cd: u16`；常量 `player::JUMP_COOLDOWN = 600`。
- 写点：C 组 `LIFE_JUMPING` 臂倒计时归零回 ALIVE 的那一帧写 `jump_cd = JUMP_COOLDOWN`
  （落地起算，两次起跳最少间隔 630 帧，照 gameplay-design §10）。
- 计时：C 组每帧 `jump_cd > 0` 则减一（任何非 GAMEOVER/ABSENT 态）。**停止冻结期间不走**。
- 门禁：`try_jump` 加 `jump_cd == 0`；冷却中按 = no-op，不计违约。
- 観測（影子世界）不改代码：冷却中影子喂的 JUMP 被门禁拒，影子自机在场，弹层照画；
  差别只在影子里自机狙瞄准在场自机——口径接受。

## 4. 死亡即遡行 + 偏差值（核 + timeline）

### 4.1 字段与删除

- `PlayerState.deaths: u8`（饱和加；纯叙事，不进任何战斗数值）。
- 删：`try_rewind`、`BTN_REWIND`（位 9，**退役不复用**）、`LIFE_RESPAWNING`（值 3 退役不复用）。
- `RESPAWN_INVULN`（120）保留，唯一消费者变为续关。

### 4.2 决死窗口耗尽 `commit_death`（C 组）

1. `lives` 饱和减一；`deaths` 饱和加一；发 `EVT_PLAYER_DIED{data=[lives,0]}`（不变）。
2. `lives == 0` → `LIFE_GAMEOVER`，**不发遡行请求**。
3. 否则：`life_state = ALIVE`、`state_timer = 0`、`invuln = max(invuln, REWIND_INVULN)`，
   位置不动；发 `EVT_REWIND_REQUESTED{a_index=i, x/y=自机位置, data=[hit_frame,0]}`。

无 timeline 的宿主（golden/run/storm/bench）到此为止 = 原地继续。有 timeline 的宿主：
`Timeline::advance` 认领逻辑**一行不改**（恢复 `hit_frame − REWIND_DEPTH` 快照 → `rewind_landed`
→ 覆写环槽 → 截 log → 记 cut）。

### 4.3 落地 `rewind_landed(i)`（签名不变 ⇒ `Cut` 结构与 `Playback` 不变；`InputLog` 仅因 §2.3 删 `Loadout.time_stops` 而 `LOG_FILE_VER` 1→2）

快照是被弹前 30 帧的世界，残机/偏差值/符卡资格都是旧值——**代价全部在落地侧重算**：

1. `deaths` 饱和加一。
2. `lives = max(lives.saturating_sub(1), 1)`（拍板 ④）。边缘：快照残机 1、死前 30 帧内吃奖命、
   再死——世界侧判「有命」发了请求，落地钳 1：奖命丢、这次死亡不收命。
3. `invuln = max(invuln, REWIND_INVULN)`（不变）。
4. 全部 active 符卡槽 `capture_ok = 0`。已知缝：符卡在死前 30 帧内才宣言 ⇒ 快照里未开卡、
   重演宣言后资格回来——窗口 0.5 s，接受，不修。
5. 越界 `i` → no-op + `contract_viol`（不变）。

### 4.4 续关

`try_continue`：`life_state = ALIVE`（原 RESPAWNING）、位置不动（原场底中心）、
`invuln = RESPAWN_INVULN`；其余（残机/库存回默认、`score = continues`、`continues+1`、
`state_timer = 0`）不变；`bomb_phase`/`bomb_timer`/`time_stops` 三行随字段删。`deaths` 不清
（偏差值是整局叙事，续关不洗白）。

## 5. 桥（`stg-godot`）

- 删常量 `BTN_TIMESTOP`、`BTN_REWIND`；加 `JUMP_COOLDOWN`、`STOP_STOCK_MAX`。
- `hud_player` 加 `jump_cd`、`deaths`。
- `new_game_at` 签名不变（`bombs` 入核后由 `new_game_at` 钳 5）；删 `time_stops` 那段注释。
- 桥级冒烟（`crates/stg-godot/smoke/`）：默认 bombs 3 → 2；遡行用例从「决死窗口按 V」改为
  「中弹后空跑到窗口耗尽 → `step_frame` 返回落点」，断言残机 −1、`deaths == 1`；续关用例
  断言 `life_state == 1`（ALIVE）。

## 6. 壳（`godot/`）

- `input.gd`：删 `stg_timestop`（D）、`stg_rewind`（V）两个 action 与 mask 位；注释改为 X = 停止。
- `play.gd`：
  - `OBSERVE_WINDOW` 60 → **180**。
  - 死亡遡行无新代码：`step_frame` 返回落点 → 现成 `_begin_rewind` 倒放。
  - `_update_time_hint`：DEATHWINDOW 文案「V 遡行」→「X 停止」；観測中 `jump_cd > 0` 时加
    「冷却中」（C 第二下照样注入，由核门禁拒）。
  - `--shots` 脚本输入：第 200 帧的 bomb 截图帧改名为停止语义（`stop_t1/t6/t14`），仍按 X。
- `hud.gd`：`Bomb` 行 → `Stop   n (碎片)`；新增跳躍冷却条（`1 − jump_cd / JUMP_COOLDOWN`，只画条）；
  新增偏差计行 `偏差   n`。
- 工程冒烟（`godot/smoke/`）：凡用到 bomb / 时停 / V / RESPAWNING 的断言按新语义改。

## 7. 测试（TDD，判别式优先）

**停止**
- deathstop：决死窗口内按 X → ALIVE、库存 −1、冻结 180；JUMPING / GAMEOVER 下按无效。
- 按住 X 跨过整个 180 帧冻结只扣一发；库存 0 no-op；冻结中再按不扣。
- 触发帧作废 active 符卡 `capture_ok`；未开卡时无副作用。
- 触碰几何判别：弹心距 = `br + hit_radius` 被消且 +10；距离 +1 raw 不消；`delay` 弹不消。
- 冻结中被消弹当帧回收、写 `VANISH_CLEARED`；未碰的弹、道具、敌、作用区逐位不变
  （改写 `full_freeze_changes_nothing_but_the_always_running_fields` 为「无触碰时逐位不变」）。
- ECL 演出冻结（`freeze_left=[0,N]`）下不跑行 8（`cutscene_freeze_still_lets_the_player_be_hit` 保留）。
- 碎片 4 进 1；库存 5 时第 4 枚清零不加；`add_bombs(+9)` 钳 5；`new_game_at` 钳 5。

**跳躍冷却**
- 落地帧 `jump_cd == 600`；冷却中按 JUMP 仍 ALIVE；冻结 N 帧 `jump_cd` 不变；恰第 600 帧后可再跳。
- 影子世界：冷却中 `preview` 的影子自机 `life_state == ALIVE`（口径钉死）。

**死亡**
- 残机 3：窗口耗尽 → ALIVE、原地、`invuln == 30`、`lives == 2`、`deaths == 1`、发请求。
- 残机 1：→ GAMEOVER、不发请求、`deaths == 1`。
- timeline：死亡被认领 → 落地世界 `lives == 快照 lives − 1`、`deaths == 快照 + 1`、active 卡
  `capture_ok == 0`；快照 `lives == 1` 边缘 → 钳 1。
- 回放：`replay_reproduces_live_checksums_including_jumps_and_rewinds` 改为靠死亡触发遡行（≥2 次），
  逐帧校验和一致。
- 续关 → ALIVE、`invuln == 120`、位置不动、`deaths` 不清。

**布局**：`PlayerState` 净变化 = −`bomb_phase`(u8) −`bomb_timer`(u16) −`time_stops`(u8)
+`jump_cd`(u16) +`deaths`(u8)——**D20 盲区警告**：`size_of` 可能不动，尺寸哨兵注释写实测值与
每个新字段的 `copy_into`（整块 `Copy`）核对理由；`jump_cd`/`deaths` 各一条「改值 ⇒ 校验和变」测试。

## 8. 版本与闸门

- `ENGINE_VER` 19 → **20**，理由串：PlayerState 布局 / 碰撞矩阵行 8 / 号表退役 513 / 词表退役位 7、9 /
  表格式删 bomb / 生死状态机退役 RESPAWNING。
- 金向量两段 md5 重钉（实测为准）；`tables_v0.bin` 重烘 + `verify-tables`。
- 全绿：`cargo fmt --check`、`clippy -D warnings`、`cargo test --workspace`、`storm`（release）、
  两个冒烟 SMOKE OK、`gen-ecl-meta` 同步无 diff；有头目验（VNC）三张：停止中触碰消弹、死亡倒放落地、冷却条。

## 9. 文档

- `docs/ecl-ops.md` 号表 513 退役；`docs/ecl-lang/` 手册（`add_bombs` 语义与钳 5、删 `add_time_stops`）+
  `gen-ecl-meta` 重生成（`7-reference.md` / `editors/vscode/stg-ecl/ecl-meta.json`）。
- `docs/render-contract.md`：`vanished` 冻结期间只出 `VANISH_CLEARED`。
- `docs/gameplay-design.md`：§2 観測按键措辞改为「按 C 観測（3 s 窗口），窗口内再按 C 跳躍」；
  §10 标注 1–4、6、7 已落地（5 待探针刀）。
- `docs/follow-ups.md`：销 **F19**（时停键占 D）。探针刀不进 follow-ups，写在 PROGRESS「之后」。
- `PROGRESS.md` 史一行 + 重写「现在」；`CLAUDE.md`：LIFE 态、M3 段里 `BTN_REWIND` 的描述同步。
- `docs/architecture.md` 如有 bomb/RESPAWNING 描述同步。

## 10. 实施偏差记录

| # | spec / plan 原文 | 实际 | 理由 |
|---|---|---|---|
| a | 删 `attract_all_items` | 保留 | 它是 `pub fn`，无 dead_code 警告；删它是无关 API 收缩 |
| b | 回放闸 1400 次推进 / 跳 ≥2 | 500 次 / 跳 ≥1 | 冷却 600 下凑两跳要 1400 次，单测 38 s（每次存环 + 全量校验和）；跳 1 次已覆盖混合回放 |
| c | `full_freeze_changes_nothing…` 挪自机 | 先抹 busy_world 预置的 `BULLET_CLEARED` | 红因是冻结分支首帧收已清除弹（spec 行为），不是触碰；性质改押「无触碰无待收弹时逐位不变」 |
| d | §4.3 落地只重算代价 | 落点快照在决死窗口时另拨回 ALIVE、清 `state_timer` | 核心复审 Important：`seal_history`/窗口内读档后落点仍在窗口 ⇒ 连环遡行至 GAMEOVER（已复现） |
| e | §4.2 `commit_death` 后 A 组照跑 | 死亡帧 A 组整段跳过 | 复审 Minor：无 timeline 宿主同帧按 X 会扣命后再扣停止；deathstop 有效窗口因 C 组先于 A 组为 7 帧（旧代码即如此） |
| f | 目验沿用 3 命 | `--shots` 下 9 命 + 每次落地左右交替闪 40 帧 + GAME OVER 响亮退出 | 落点 = 被弹前 30 帧且落地无敌 30 帧、脚本输入按帧号固定 ⇒ 确定性连死，停在 GAME OVER 页挂到超时 |
| g | 2P 多请求 | 未处理，记 follow-ups **F25** | 同帧双死只认领一条，第二人代价被快照抹掉；现无 2P 内容 |

实测：`ENGINE_VER` 20；金向量 md5 `15a5167cfadd7e8a4c835b26bc5e8b92`（段一自帧 0 起变，段二逐字节不变——T6 时的
`680fb23e…` 被 e 条再改一次）；词表指纹 `0xFE04_C7CD_7FE1_0485`；`PlayerState` `size_of` 全程 72（三删两加全被
padding 吸收，尺寸哨兵未响，靠两条「改值⇒校验和变」测试押）；测试 核 673 + 编译器 327 + 桥 17 + harness 47；
storm ✔、verify-tables ✔、两冒烟 SMOKE OK、`--shots` 有头目验 11 张（含 `stop_t*`/`rewind_land`/`boss`）exit 0。
