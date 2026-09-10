# 壳子刀 —— 游戏流程壳：标题 / 难度 / 关间结算 / 续关 / 练习 / 回放（设计，2026-09-11）

> 状态：**已拍板，直接实施**（用户要求不走完整 superpowers 执行流程；实施偏差见 §8）。
> 前置：转场协议修正（`stage_clear()` + `seal_history`，2026-09-07）；时间机制内核刀（timeline）。
> 玩法来源：`docs/project_overview.md` §2.2（残机制度保留）、§2.9（练习模式最小版）、§4.7
> （不续关通关解锁 EX）。

## 1. 范围（最小闭环）

标题（开始 / 练习 / 回放 / 退出）→ 难度选择（四档）→ 游玩 → 每关 `EVT_STAGE_CLEARED{N}`
结算页 → 下一关 → `stage_clear(0)` 结果页 → 回标题；GAME OVER 页（续关 / 回标题）；练习模式
（`mark` 入口 + 难度）；结果页与 GAME OVER 页可存回放；标题「回放」列出 `user://replays/*.stgr`
并播放。**不做**：设置页、成绩榜、对话、结局分支、偏差值显示。

## 2. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | Godot 骨架 | **A**：`GameFlow` 状态机 + 页面切换；游玩页常驻、结算/GAME OVER/结果做覆盖层；回放复用游玩页 |
| ② | 续关 | **A**：输入位 `BTN_CONTINUE`，世界内做续关（残机回默认、分数 = 续关次数、`continues+1`、重生）；可回放、不加写 API |
| ③ | 内容目录 | `res://ecl/game/`，demo 整体搬为第 1 关；`mark` 号约定 道中 `N*10`、boss `N*10+5`；`stage_clear(0)` = 结局 |

## 3. 内核

- `BTN_CONTINUE = 10, Edge`。相位 3 循环头：`LIFE_GAMEOVER` 下 `try_continue(i)`——上升沿即
  `lives/bombs/time_stops` 回 `Loadout::default()`、`score = continues + 1`（东方惯例）、
  `continues += 1`（饱和）、走 `LIFE_RESPAWNING`（同 `commit_death` 的重生分支：场底中心 +
  `RESPAWN_INVULN`）。power 不动。`PlayerState.continues: u8` 进校验和。
- 回放播放：`timeline::Playback { log, cursor, cut_i }`；`Timeline::start_playback(log, image)`
  从 `log.boot` 开机；`playback_step(&mut Playback) -> Result<PlaybackStep, ReplayError>`：先落
  当前帧的 cut（`rewind_landed` + 覆写环槽 + 记 cut），再喂 `frames[cursor]`；到末尾 `done`。
  `replay_with` 改为这条循环的包装，两者不可能分歧。线性 log 里出现遡行请求仍是 Err。
- `ENGINE_VER` 18 → 19（新字段 + 新输入位）。金向量预期不变（字段被 padding 吃掉且风铃卡不按键）
  ——实测为准。

## 4. 桥

- `new_game_from_replay(names, sources, bytes) -> bool`：编译 → `InputLog::from_bytes` 头校验
  → `start_playback`；`Game` 持 `playback: Option<Playback>`。
- `playback_step() -> i64`：-2 播完 / -1 正常 / ≥0 落地帧（播放里没有被丢弃的分支，壳不做倒放
  动画，直接切画面）。上传三层同 `step_frame`。`is_playback() -> bool`。
- 常量：`BTN_CONTINUE`、`LIFE_GAMEOVER`；`hud_player` 加 `continues`。

## 5. 壳

```
main.tscn (GameFlow: main.gd)     流程状态机：TITLE / DIFFICULTY / PRACTICE / REPLAYS / PLAY
  Title / Difficulty / Practice / ReplaySelect   代码生成的 Control 菜单（无 tscn）
  Play (play.gd)                    现 main.gd 游玩部分原样搬入：bridge/playfield/hud/effects/
                                    dispatcher/観測遡行/--shots/冒烟；start(mode, rank, mark, bytes)
    Overlay (overlay.gd, CanvasLayer)  结算页 / GAME OVER 页 / 结果页 / 播完页
```

- 关间：`EVT_STAGE_CLEARED{N}` → `seal_history()` → `stop stepping` → 结算页（关号、分数、graze、
  续关数）→ Z 确认 → 恢复。`N == 0` → 结果页（总分、续关数、`continues == 0` ⇒ 「EX 解锁」字样、
  Z 存回放 / X 回标题）。练习模式收到任何 `EVT_STAGE_CLEARED` → 回练习菜单。
- GAME OVER：`life_state == GAMEOVER` → 页；Z 续关 = 注入一帧 `BTN_CONTINUE` 后恢复；X 回标题；
  播放模式禁续关（日志里有就照播）。存回放两页都有（C 键），文件 `user://replays/<unix>_r<rank>.stgr`。
- 回放播放：Play 的 step 回路把 `step_frame(buttons)` 换成 `playback_step()`；播完盖「播完」页。
- 顺手：F20（fx 按出生帧清理）、F23（自机动画帧：壳侧按 `(facing, 输入, 帧号)` 选帧，核零改动；
  命名对齐与 `WorldView` 读口本刀**不做**——只做壳侧帧表，留 F23 余项）。
- 键位：菜单 方向键 + Z 确认 + X 返回。

## 6. 内容搬家

`godot/ecl/demo/` → `godot/ecl/game/`（git mv），`main.ecl` 的 `mark(1)/mark(2)` 改 `mark(10)/mark(15)`，
末尾 `stage_clear(1); stage_clear(0);`（demo 只有一关：结算页之后直接结果页）。`content_tables.gd`
加 `PRACTICE = [{name, mark}]`。所有引用 `ecl/demo` 的路径（`main.gd`、harness `replay.rs` 测试、
文档）改 `ecl/game`。

## 7. 测试与闸门

- 核：续关判别（GAMEOVER 按沿 → 残机/分数/计数/重生；ALIVE 按沿零变化；饱和）；`playback_step`
  逐帧播完末态 == `Timeline::replay`，遇 cut 落地。
- 桥级冒烟：续关；`new_game_from_replay` 播完末态校验和 == 录制时。
- 工程冒烟：标题 → 难度 → 开局；内联 ECL（`wait; stage_clear(1); wait; stage_clear(0)`）走
  结算页 → 确认 → 结果页；GAME OVER → 续关 → ALIVE。既有断言全部保留（Play.run_smoke）。
- `--shots`：结算页一张（根视口截图，覆盖层在 SubViewport 之外）。
- 全套：fmt / clippy / test / storm / verify-tables / 两冒烟 / 金向量 md5。

## 8. 实施偏差记录（2026-09-11 收口）

| # | spec 原文 | 实际 | 理由 |
|---|---|---|---|
| a | §3「`continues` 落进 1 B 空档，`size_of` 不变」 | `PlayerState` 64→72，`World` +16 B，尺寸哨兵响 | 上一刀 `hit_frame` 已把尾部 padding 吃光；哨兵这次工作了。金向量 md5 `978522fd…` |
| b | §5 顺手做 F23 壳侧帧表 | **未做** | `player.png` 32×32 只有一格，没有帧可选；等美术 |
| c | §7 `--shots` 结算页一张 | 未得 | llvmpipe 下常按射击 400 s 内打不死 demo boss；功能由工程冒烟内联脚本覆盖（结算页/结果页/GAME OVER/续关/存回放/播放） |
| d | §5 菜单为 Control 场景 | 代码生成的 `Menu` CanvasLayer，无 tscn | 四个菜单同一形状，一个类够用 |
| e | §5 回放播放时 GAME OVER 页 | 不盖页、不停拍 | 录制时的停拍贡献零帧，播放照走；日志里若有续关照播；播完才盖页 |
| f | 提交切分 | 核 / 桥 / 壳+内容搬家+文档 三段 | 同 spec §7 |
