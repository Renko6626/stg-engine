# 时间机制内核刀 —— timeline + 観測/跳躍/遡行（设计，2026-09-07）

> 状态：**已实施收口（2026-09-07，三段提交：核 0804508 / timeline 43593a6 / 桥壳 a2da229）**，
> 实施偏差见 §11。本文是 brainstorming 环的产出，口径由人类逐条裁定（§10「人类拍板记录」
> 留痕）。按用户要求未走完整 superpowers 执行流程（无独立 plan 文件），直接按 §7 三段实施。
> 玩法来源：`docs/setting.md`（東方時環譜 v0.5）§2.1/§2.3/§2.5/§7.6。
> 本刀**重新定义 M3**：原「环形快照 + 输入扰动 harness（本地回滚）」改为「时间机制内核刀」，
> 联机回滚是附带收益不是目标。

## 1. 是什么 / 为什么

策划案的四件时间工具里，**停止**已由自机能力刀落地（时停 + bomb，合并口径留后），本刀做
剩下三件的**核心机制**，资源、库存、偏差值一概不做（§9 非目标）：

| 机制 | 玩家看到的 | 引擎里是什么 |
|---|---|---|
| **観測** | 按一下 C，屏幕上叠出「从此刻起我什么都不做，N 帧后弹幕在哪」的暗色影子 | 宿主侧克隆权威世界跑 N 帧的**影子世界**，纯表现，不碰权威状态 |
| **跳躍** | 観測窗口内再按一下 C，影子瞬间变成现实，自机跳过了那 N 帧 | 一个沿触发输入位 + 一个新生命态 `LIFE_JUMPING`，**完全住在 World 里** |
| **遡行** | 被弹后的决死窗口内按 V，画面逐帧倒退回被弹前半秒 | World 之上的 **`timeline` 模块**：快照环 + 恢复 + 落地补写 + 回放日志 |

三条设计基石：

1. **「预览即真实」靠同一份代码，不靠约定。** 影子世界的模拟 = 克隆 + 喂一帧跳躍位 + step N，
   与真跳走**同一条代码路径**；确定性（I1–I7）保证两者逐位相同。
2. **World 对时间线无知**（P1/P5）。World 只多两个小口：`try_rewind` 发请求、`rewind_landed` 收
   落地。一切代价都在**落地侧**付，请求失败时世界零变化。
3. **回放格式与节拍无关。** timeline 永远一帧一帧，「跳躍瞬时完成」是宿主在一个 tick 里连续
   `advance` 的节拍选择；回放日志里就是 N 帧空输入，世界自己把自机当缺席。

## 2. 世界侧改动（`stg-core`，断层线以下，全部进校验和）

### 2.1 输入位

`input.rs` 注册表加两行：

```
BTN_JUMP   = 8, Edge;   // 跳躍（消费者：world/player.rs::try_jump）
BTN_REWIND = 9, Edge;   // 遡行请求（消费者：world/player.rs::try_rewind）
```

位为 0 等价旧行为，旧回放不破（注册处既有纪律）。**観測不进世界**：它免费、无副作用，两次按键
的协议由宿主管，宿主只在第二下把 `BTN_JUMP` 送进来一帧。

### 2.2 `LIFE_JUMPING`（新生命态，`player.rs`）

`pub const LIFE_JUMPING: u8 = 5;` **不复用 `LIFE_ABSENT`**——ABSENT 连 C 组计时都跳过，跳躍需要
倒计时。语义（相位号见 §3.5）：

| 相位 | 行为 |
|---|---|
| 3 C 组 | `state_timer` 从 `JUMP_FRAMES` 倒数，到 0 置 `LIFE_ALIVE`。场景冻结时不走（与其他 C 组计时同口径） |
| 3 A 组 | **整段跳过**：不动、不射、不用能力（时停/bomb/跳躍/遡行都不响应） |
| 6 | 跳过该自机的受击判定与擦弹判定 |
| 7 | 跳过该自机的道具拾取（含近距磁吸的自机选择） |
| 瞄准 | `nearest_aimable_player` 的排除名单加 `LIFE_JUMPING`（现只排 ABSENT/GAMEOVER）。`aim_target` 回退到最后坐标——正是「我什么都不做」的语义。单人局下与不排除逐位相同（坐标未动），排除是为语义一致 |

**门禁 `try_jump(i)`**：仅 `LIFE_ALIVE` 且 `!scene_frozen()` 时响应沿；DEATHWINDOW /
RESPAWNING / JUMPING / 时停中按下 = no-op（P4-b，不计违约——这是玩家操作不是脚本坏参）。
进入时 `state_timer = JUMP_FRAMES`。两种时间能力不叠加，规则只有一条。

**常量**：`pub const JUMP_FRAMES: u16 = 30;`（`player.rs`，与 `TIMESTOP_FRAMES` 并列），进
`consts.rs` ① 段注入 ECL。

### 2.3 遡行的两个小口

- **`PlayerState.hit_frame: u32`**（新字段）：进入 `LIFE_DEATHWINDOW` 那一帧写入 `World.frame`
  当帧值。自动进校验和/存档（derive）。
- **`try_rewind(i)`**（A 组，`try_bomb` 之后）：仅 `LIFE_DEATHWINDOW` 内响应沿；只发一条事件
  `EVT_REWIND_REQUESTED = 10`，载荷 `{player: i, hit_frame}`；**不改任何状态**。ALIVE 按下 = no-op。
  与 deathbomb 的先后：同一帧既按 bomb 又按遡行，`try_bomb` 先跑并把状态拨回 ALIVE，`try_rewind`
  随之门禁不过——bomb 优先，写进文档。
- **`pub fn rewind_landed(&mut self, i: usize)`**（`world/player.rs`，写 API）：timeline 恢复快照后
  调用。写 `invuln = REWIND_INVULN`（常量 30），断言 `life_state == LIFE_ALIVE`（落点在被弹前，
  必为 ALIVE；不是则 P4-c debug 断言）。**将来库存扣一、偏差值加一都从这一个口进**。

### 2.4 尺寸与版本

- `hit_frame` 是 u32，`PlayerState` 对齐空档只剩 1 字节（follow-ups D20），`size_of` 必变 →
  尺寸哨兵会响，按四件套清单走（`copy_into` / checksum derive 自动 / D10 容量预算 / 存档格式
  derive 自动）。
- `ENGINE_VER` 16 → 17：新字段、新生命态、新输入位、新事件号，旧存档/回放拒载。金向量 md5
  预期改变（全槽哈希多了字节），三平台 CI 对拍。

## 3. `stg_core::timeline` 模块

住 `step.rs` 同层（组装层之上），无浮点无 Godot 无时钟，harness 与桥共用。World 对它无知。

### 3.1 结构

```rust
pub struct Timeline {
    world:  Box<World>,
    ring:   SnapshotRing,      // RING_DEPTH=48 个 Box<World> 槽，开局分配一次，之后只 copy_into
    shadow: Box<World>,        // 影子世界，preview 专用
    log:    InputLog,
    tables: &'static WorldTables,
    image:  EclImage,
}
pub struct SnapshotRing { slots: Vec<Box<World>>, /* frame 号 → 槽 = frame % DEPTH */ oldest: u32, newest: u32 }
pub struct InputLog { boot: Boot, frames: Vec<InputFrame>, cuts: Vec<Cut> }
pub struct Cut { at: u32, to: u32, player: u8 }   // at 只作诊断；重放只看 to
pub enum Boot { NewGameAt { loadout, mark, seed }, Snapshot { save_hash: u64 } }
pub struct Advance { pub rewound: Option<Cut> }
```

常量：`RING_DEPTH = 48`（落点 = 被弹帧 − 30，按键最晚在被弹后 8 帧，38 + 余量）、
`RING_STRIDE = 1`（步长 2 减半内存；恢复逻辑按步长通用实现：就近取 ≤ 目标的存档帧，再用 log
补 step 到目标）、`REWIND_DEPTH = 30`（进 ECL 常量注入）。

内存账：48 × ~1.07 MB ≈ 51 MB 环 + 1 MB 影子。`Box<World>` 分配只在构造时发生（F1 纪律：
不新增 std 触点）。

### 3.2 `advance(&mut self, input: &InputFrame) -> Advance`——一次只走一帧

1. `step(world, input)`；`log.frames.push(input)`；若 `frame % RING_STRIDE == 0` 则 `world.copy_into(ring[frame])`。
   索引约定：`log.frames[f]` 是 `world.frame() == f` 时喂的输入，产出状态 f+1。
2. 扫 `world.frame_events()`，遇 `EVT_REWIND_REQUESTED{p, hit_frame}`：
   - 落点 `F = max(hit_frame.saturating_sub(REWIND_DEPTH), ring.oldest)`——**始终钳位**。环在构造
     时就把初始世界压进去当帧 0（`from_world` 同样把载入的世界压进去），所以环永不空、请求
     永远能兑现；开局第 5 帧被弹就退到帧 0，是一次短遡行而不是失败。
   - 恢复到 F（步长 > 1 时从就近存档帧用 `log.frames` 补 step），调 `world.rewind_landed(p)`，
     **把落地后的世界覆写回环槽 F**（环永远是「现在这条历史」），环里比 F 新的槽作废
     （`newest = F`），`log.frames.truncate(F)`，`log.cuts.push(Cut{at, to: F, p})`，返回
     `rewound: Some(cut)`。
3. 一帧内最多一条请求（单自机）；多自机时按 player 升序取第一条，其余忽略（I4）。
4. 「请求失败时世界零变化」（§1 基石 2）在本刀没有失败分支，是给将来库存门禁留的规则：
   届时库存为 0 的请求在 `try_rewind` 就不发事件。

**跳躍的快进不放在 timeline 里。** 宿主看到 `life_state == LIFE_JUMPING` 就在一个 tick 里连续
`advance`（喂空帧）直到回 ALIVE，最多 `JUMP_FRAMES + 1` 次。timeline 永远一帧一帧，「瞬时还是
动画」归表现层，回放格式与节拍无关。

### 3.3 影子世界 `preview_begin()` / `preview_step() -> &World`

- `preview_begin()`：`world.copy_into(shadow)`；清掉 `shadow.body.players[0].prev_input` 的
  `BTN_JUMP` 位（防玩家正按着键导致沿检测不到；timeline 在 crate 内，可走 `pub(crate)`）；
  第一步喂 `InputFrame::empty(frame)` 加 `actions[0].buttons = BTN_JUMP`。
- `preview_step()`：之后每步喂空帧，返回 `&shadow`。桥自己决定取第几步编码，v1 只取第
  `JUMP_FRAMES` 步。
- **不碰环、不碰 log、不碰权威世界**（测试押：前后三者校验和不变）。
- 性能账：每真实帧一次 1 MB memcpy + N 次 step，N=30 重弹幕下约 3–5 ms。手感要 1 秒再把
  `JUMP_FRAMES` 提到 60，届时如 profiler 指认，方向是隔帧重分叉或只编码弹层。

### 3.4 回放 = 线性 log + cuts，被丢弃的分支不保留

`Timeline::replay(boot, &InputLog) -> Result<Timeline, ReplayError>`：从 `boot` 构造，按
`log.frames` 逐帧 `advance`；走到 `cuts` 里某条的 `to` 帧（即 `world.frame() == to` 时、喂该帧
输入之前）就调一次 `world.rewind_landed(p)` 并同样覆写环槽。同一 `to` 有多条 cut（两次遡行落在
同一帧）就按记录序各应用一次——现场也是恢复「已落地一次的环槽」再落地一次，逐位同。重放不
需要重跑死掉的分支——它对未来的唯一影响就是 `rewind_landed` 那一笔。

**`InputLog` 字节格式 v1**（沿 `save.rs` 纪律：小端、无 padding、载荷 FNV）：

```
magic "STGR" | file_ver u8=1 | ENGINE_VER u32 | tables_hash u64 | image_hash u64
| boot (tag u8 + 定长载荷) | n_frames u32 | frames[n] (定长 InputFrame)
| n_cuts u32 | cuts[n] (at u32, to u32, player u8) | payload_fnv u64
```

`Boot::Snapshot` 的 log **不可从开局重放**（`replay` 返回 `ReplayError::BootNotReplayable`），
只能落盘与展示；练习模式再把快照嵌进去（follow-ups 记档）。

### 3.5 读口

`world()`、`frame()`、`ring_get(frame) -> Option<&World>`（宿主倒放用）、`log()`、
`log_bytes() -> Vec<u8>`、`Timeline::from_world(world, boot)`（`load_state` 后重建：环清空、log 重开）。

## 4. 桥面（`stg-godot`）

- `boot.rs::Game` 的 `world: Box<World>` 换成 `timeline: Timeline`。
- `step_frame(buttons) -> i64`：调 `advance`；正常返回 -1，发生遡行返回落点帧 F。宿主看
  `hud_player().life_state == LIFE_JUMPING` 决定是否在本 tick 继续调，桥不管节拍。
- 常量导出补 `BTN_JUMP`、`BTN_REWIND`、`LIFE_JUMPING`、`JUMP_FRAMES`、`REWIND_DEPTH`，走现有
  `BTN_*`/`LAYER_*` 同款镜像 + 对拍测试。
- **影子读口 `preview(n: i64) -> bool`**：跑影子 n 步，把影子的弹层编码进新注册层
  `LAYER_GHOST_BULLETS`（stride 12 布局不变，复用 `frame.rs` 编码器，数据源换成影子世界）。
  v1 只给弹层；敌人木偶的未来态不做（follow-ups 记档）。
- **倒放读口 `view_ring(frame: i64) -> bool`**：把环里那一帧的三层 + 木偶喂料重新编码上传。
  所有通道 A 读口（层缓冲、`puppets()`、`hud_*`、`anchors()`）改成从「当前视图世界」取：默认
  = 权威世界，`view_ring` 临时切过去，下一次 `step_frame` 自动切回。`vanished()` 在视图态返回空。
- `load_state` 之后 `Timeline::from_world(.., Boot::Snapshot{hash})`。
- 新导出 `replay_bytes() -> PackedByteArray`。回放校验在 harness 侧（`harness replay <file>`）。
- 桥级冒烟（`crates/stg-godot/smoke/`）加三条：① 按跳躍位后 `life_state == LIFE_JUMPING`，31 次
  `step_frame(0)` 后回 ALIVE；② `preview(30)` 后影子层 `visible_instances` 非零；③ 构造决死
  窗口后按遡行位，`step_frame` 返回的 F 等于被弹帧 − 30 且 `frame()` 回到 F。

## 5. Godot 壳（`godot/`）

- **键位**（`input.gd`）：C = 観測/跳躍，V = 遡行，时停临时挪到 D，X bomb 不动。停止合并那刀再收。
  `mask()` 加 V → `BTN_REWIND`（世界侧沿检测 + 决死窗口门禁够用）；**不碰 `BTN_JUMP`**。
- **観測两段协议**（`main.gd` 小状态机）：`IDLE` 按 C 进 `OBSERVING`，窗口 `OBSERVE_WINDOW = 60`
  tick，期间每 tick `bridge.preview(JUMP_FRAMES)`、显示影子层；窗口内再按 C 就把 `BTN_JUMP`
  注入**这一帧**的输入，回 `IDLE`；窗口到期回 `IDLE`、隐藏影子层。JUMPING/REWINDING 期间按 C 忽略。
- **快进**：`_physics_process` 里 step 之后，`while life_state == LIFE_JUMPING and n <= JUMP_FRAMES:
  step_frame(0)`，`_after_step` 只跑最后一次。中间帧的通道 B 请求与 `vanished` **丢弃**——她不在场，
  「线框态瞬时对齐为实体」。
- **影子层**（`playfield.gd`）：第四张 MultiMesh，图集同弹层，`ghost.gdshader` 是 `layer.gdshader`
  变体：低饱和、比实弹暗、半透明（策划案 7.4「预测永远比实体暗」）；真线框等美术。
- **遡行倒放**：`step_frame` 返回 F ≥ 0 进 `REWINDING` 表现态：每 tick `view_ring(f)`，f 从 G−1 按
  `SCRUB_STEP = 3` 往回跳到 F，期间不 step、不收输入。到 F 后三件事：`dispatcher` 水位重置到 F、
  fx 池整个清空（落地后出生的行都不该活着）、`_sync_anchors()` 把 HUD 与背景电平拉回 F
  （这正是 follow-ups A7 记的「读档后必须对表」场合，A7 的 `_sync_anchors` 复用不变）。恢复出来
  的世界里的请求缓冲不再分发，水位挡住。
- **HUD**：観測中的指示 + 窗口条；决死窗口内提示「V 遡行」。库存、偏差值不做。
- **目验与冒烟**：`--shots` 脚本化输入加两段（観測→跳躍、受击→遡行），截图看影子层与倒放；
  `godot/smoke` 加三条断言，对应 §4 桥级那三条。
- **文档**：`render-contract.md` 加影子层（§3.8）、倒放规矩（水位重置 / fx 清空 / A 通道读环，
  §5.6）、§7 清单两项。

## 6. 测试与闸门

行为正确性靠单测守，金向量只抓跨平台分歧（CLAUDE.md「金向量闸门的能力边界」），每条招牌语义
配**判别式**测试。

**核内**

- 输入：两个新位进 `EDGE_MASK`；词表指纹测试随之更新。
- `try_jump`：ALIVE 按下进 JUMPING 且 `state_timer == JUMP_FRAMES`，恰好 N 帧后回 ALIVE（N±1 红）；
  DEATHWINDOW / RESPAWNING / JUMPING / 场景冻结下按下零变化。
- 缺席语义五条，每条配 ALIVE 对照组：弹压在自机上不判中、道具在拾取圈内不吃、按射击不出弹、
  按方向不动、擦弹不计；`aim_target` 在 JUMPING 下回退到最后坐标。
- `try_rewind`：只在 DEATHWINDOW 发事件且 `hit_frame` 等于中弹帧；ALIVE 按下零事件；同帧
  bomb + 遡行 → bomb 赢、零事件。`rewind_landed`：`invuln == REWIND_INVULN`、ALIVE。
- 尺寸哨兵更新；`copy_into` 字段清单测试；存档往返。

**timeline**

- 环：推入/取出/最老槽淘汰；步长 2 时就近恢复 + 补 step 与步长 1 逐位同。
- **预览等于真跳**：影子走 N 步的校验和 == 权威世界喂一帧 `BTN_JUMP` 再走 N 帧空帧；并断言
  != 不跳的世界（防空转）。`preview` 前后权威世界、环、log 校验和不变。
- 遡行：第 H 帧构造中弹、H+3 按键，断言 `frame() == H−30`、校验和 == 环槽 H−30 加落地写、
  log 长度 H−30、cut 记录、`ring.newest == F`。开局第 5 帧中弹 → 钳到帧 0（校验和 == 初始世界
  加落地写）。同一落点两次遡行 → 重放逐位同（押 §3.4 那句）。
- **回放闸**：随机输入 + 脚本化跳躍 + 强制中弹 + 遡行混跑数百帧，录 log，`Timeline::replay`
  从开局重放出逐帧相同的校验和流；`log_bytes` 往返全等；`Boot::Snapshot` 拒重放。短版进单测，
  全量进 harness 命令 `harness replay <file>`（storm 的时间线版）。

**外围**

- `JUMP_FRAMES` / `REWIND_DEPTH` 进 ECL 常量注入，`gen-ecl-meta` 重跑同步三个 sink。
- 金向量：`ENGINE_VER` 17 配新 md5，三平台 CI 对拍。
- 桥级冒烟三条、Godot 冒烟三条、`--shots` 目验两段截图；fmt / clippy / storm / verify-tables 全绿。
- 文档：`PROGRESS.md` 与 CLAUDE.md 的 M3 定义改成「时间机制内核刀」；`follow-ups.md` 新记
  （§8）；`render-contract.md`、`ecl-lang` 常量表、`ecl-ops.md` 事件号同步。

## 7. 提交切分

沿表现契约 v2 刀的三段式：① 核（输入位 / JUMPING / 遡行两口 / `ENGINE_VER` / 金向量重 bless）
② timeline 模块 + harness `replay` 命令 ③ 桥 + 壳 + 冒烟 + 目验；文档单独一提交。

## 8. follow-ups 的记

| 条目 | 内容 | 触发点 |
|---|---|---|
| 余晖 | 弹的半秒拖尾（策划案 7.5）——环里就是历史，表现层直接 `ring_get` 或壳侧留最近 N 份实例缓冲 | 美术期 |
| 敌人未来态 | 影子层 v1 只有弹；敌人木偶的线框态要第二套 256 个 Sprite2D 或改 MultiMesh | 观测要看敌人走位时 |
| 跳过帧的 B 请求 | 快进丢弃中间帧请求与 `vanished` 是有意口径；若要「跳躍落地时补一次爆炸」需 timeline 攒事件 | 演出打磨 |
| `Boot::Snapshot` 不可重放 | `load_state` 后 log 只能落盘；练习模式要把快照嵌进 log | 练习模式刀 |
| 键位 D | 时停临时占 D，停止合并（时停 + bomb 一份库存）时收 | 停止合并刀 |
| 影子层性能 | N=60 或重弹幕若 profiler 指认：隔帧重分叉 / 只编码弹层 / 影子跳过相位 6–7 | profiler 指认时 |

## 9. 非目标

- 遡行库存、偏差值、观测冷却、教学免费标记——资源体系（策划案 2.2）整包留后，`rewind_landed` 是它们的入口。
- 停止合并（时停 + bomb → 一份「停止」）。
- 随时可按的遡行（拍板 A，只在决死窗口）。
- 练习模式的长时间线 / 手动存档点 / 时间轴 UI——格式方向已定（关键帧 + 本 log）。
- 影子世界的多条未来态（策划案 7.6 后期）——需要分歧源，另立 spec。
- 联机回滚（M4）——本刀的环与 `advance` 语义按回滚可用写，接入不在本刀。

## 10. 人类拍板记录

| # | 问题 | 拍板 |
|---|---|---|
| ① | 観測期间权威世界冻结还是照常演化 | **照常演化（B）**：影子演「从此刻起我什么都不做」，预测弹幕轨迹够用；慢放看手感 |
| ② | 影子自机规则 | **完全缺席**，与跳躍同规则，预览即真跳 |
| ③ | 遡行触发 | **只在决死窗口内（A）**，落点被弹帧 − 30 |
| ④ | 时间线归属 | **`stg_core::timeline` 模块（A）**，World 无知；长时间线（练习模式存读档）将来也住这里 |
| ⑤ | 観測/跳躍按键协议 | 按一下进観測，窗口内再按一下跳躍（用户提出） |
| ⑥ | 快照步长 | 做成常量，先 1 后调；用户判断隔帧够用 |
| ⑦ | 资源/偏差值 | 之后再改，先核心机制 |

## 11. 实施偏差记录（2026-09-07 收口时补）

| # | spec 原文 | 实际 | 理由 |
|---|---|---|---|
| a | §2.3 `rewind_landed` 断言落点 ALIVE、写 `invuln = REWIND_INVULN` | 不断言，`invuln = max(invuln, REWIND_INVULN)` | 回放闸实测：被弹前 30 帧内自机可能正在跳躍或刚重生（`RESPAWNING` 自带 120 无敌），落点不是 ALIVE 是正常态。JUMPING 下 C 组不减 invuln，落地回 ALIVE 后才数，等于"跳完再送 30 帧"。 |
| b | §3.2 落点"钳到 `ring.oldest`"+ 步长 > 1 时"就近恢复再补 step" | 落点先向下取整到步长倍数再钳位，永远是存档帧，无补 step 路径 | 少一条只在步长 > 1 才活的代码路径；步长 2 时落点误差 ≤ 1 帧，可接受。 |
| c | §3.1 `Boot::Snapshot { save_hash }` | `Boot::Snapshot { world_checksum }` | 桥侧拿不到存档载荷 FNV（`fnv1a64` 是 core 私有），载入世界的校验和同样唯一且现成。 |
| d | §3.1 环槽淘汰"比 F 新的槽作废" | `truncate_to` 只动 `newest`，作废槽内容保留、经 `get_discarded` 可读到下一次 `advance` | 壳侧倒放动画要从请求帧 G 倒着读到 F，那些帧正是被丢弃的分支；下一 step 起同槽被新帧覆写，语义不变。 |
| e | §3.3 `preview(n)` = `n` 步 | `preview(n)` = 起跳那一步 + `n` 步，`n = JUMP_FRAMES` 时返回落地那一帧 | 首版按 spec 走 30 步，影子停在"还剩 1 帧"的 JUMPING；改成 n = 跳过的帧数，与真跳"喂一帧 JUMP 再走 n 帧空帧"一一对应，测试 `preview_equals_a_real_jump` 押运。 |
| f | §4 `LAYER_GHOST_BULLETS` 新注册层号 | 独立 `register_ghost_layer(rid)`，不占 `LAYER_*` 号 | `step_frame` 对 `layers[]` 无条件从权威世界编码上传，影子层若混进去每帧会被权威世界覆写；独立持有更直白。 |
| g | §6 桥级冒烟"构造决死窗口" | 自机上到 y≈300 再向左走进 `godot_smoke.ecl` 的弹流，真中弹 | GDScript 没有世界写口，构造不了；走进弹流反而是端到端的真判别（含 `hit_frame` 读口）。 |
| h | §6 有头目验"受击→遡行"截图 | 本次无 `rewind_land` 张 | `--shots` 的脚本化输入没撞上弹；遡行的数值判别在桥级冒烟，倒放视觉留下次目验。 |
| i | §7 三段提交 | 同；文档单独一提交 | — |
| j | 流程 | 未写 `docs/superpowers/plans/`，spec 拍板后直接三段实施 | 用户要求（"计划简单写，直接开干"）。 |

