# 表现契约 v2 —— 电平/边沿分类 + 逐实体表现计时 + 敌人木偶（设计，2026-09-07）

> 状态：**已评审拍板，待写实施计划**。本文是 brainstorming 环的产出，口径由人类逐条裁定
> （§9「人类拍板记录」留痕）。实施计划另起 `docs/superpowers/plans/`。

## 1. 是什么 / 为什么

本刀不加新玩法，**把表现层与模拟核之间的契约补全到"回滚、读档、中段开机下闭合"**，
并把 ZUN ANM 系统承担的那部分职责在本架构里各归其位。

起因是两轮调研（Bevy Extract / Factorio prepare / Overwatch / GGPO 类 / ZUN ANM）得出的结论：

- 本仓"通道 A 状态视图 + 通道 B 离散请求"的两通道模型与业界同构，**思路对**。
- 但**分类规则没写死**：什么该走电平、什么该走边沿，只在 `render-contract.md` §5 对
  BGM/BG 三个 id 做了特例（`anchors()` 双表示），没有升格成总规则。以后 boss 在场、
  受击闪白、时停滤镜、符卡背景这类"持续超过一帧、真值在核里"的状态一旦走边沿，读档
  和回滚就会撒谎。
- **通道 A 承载的逐实体数据太少**：敌池已有 `hit_flash` / `anm_state` 两个字段（进校验
  和、随快照走），但编码器一个都没往 custom 槽送；弹池连弹龄都没有，出现闪光和消弹淡出
  无从画起。
- **壳侧没有执行器**：通道 B 的消费端是七个 lambda 各写各的 tween，`effects.gd` 每个火花
  一个带 `_process`/`_draw` 的节点，擦弹一接就炸（follow-ups B28）。

### 1.1 ANM 在本架构里的落点（本刀的核心判断）

ZUN 的 ANM 是**每精灵一台微型协程 VM**，既是动画器也是绘制路径。它长成 VM 是因为 ZUN
没有引擎、DX8 固定管线没有可编程 shader。把 ANM 拆开是三样东西，本架构里各有更便宜的归宿：

| ANM 成分 | 归宿 | 理由 |
|---|---|---|
| 计时器（弹龄、受击闪、状态切换时刻） | **核**，作为不解释的整数字段 | 确定性、进校验和、随快照回滚，headless 也可观测 |
| 属性曲线（缩放/alpha/闪白随 t） | **shader**，以 `t` 为自变量 | CPU 零成本，回滚/读档/帧率解耦自动正确 |
| 编排（boss 登场、cut-in） | **Godot 原生**（AnimationPlayer/场景） | 个位数数量级，编辑器可视化 |

**不做 ANM VM**（拍板 ①）。ZUN 的 `anmInterrupt` 在本架构里 = ECL 写 `anm_state` 电平并
盖帧号，壳侧下一帧读到电平变了就播过渡。"状态不变但播一次反应"走带句柄的事件/请求。

**不做 `stg-present` 中立层**（拍板 ③，曾提出后撤回）：契约写进文档，代码留在
`stg-godot`；换渲染器时按 §7 的"必须渲染项清单"抄一遍几百行壳代码，比维护一个中立
crate 便宜。粒子特效同理**不进 Rust**（拍板 ②）：Godot 有 GPUParticles2D，任何值得换的
渲染器都有粒子系统，headless/RL 对粒子毫无兴趣。

## 2. 总规则（写进 `render-contract.md` §0）

1. **电平走通道 A，边沿走通道 B。** 边沿可丢（cap 满确定性丢弃）可重放（回滚重演），
   **电平不可丢**。持续超过一帧、且真值在核里的表现状态，一律每帧从 `WorldView`/读口拉电平。
2. **边沿只允许三类用途**：即发即忘（爆炸、火花、音效、飘字）、须确认（符卡结算、过关、
   boss 登场；M3 后只在越过回滚地平线后播）、电平镜像（BGM/BG/BG_PHASE；每个必须配一个
   电平读口，宿主开机/读档后先读读口对表）。
3. **表现层唯一时间源是 step 结束后的 `frame`**。木偶动画位置、shader 里的 t、依附特效的
   寿命全部由帧号推导；壁钟时间只允许即发即忘的 GPU 粒子使用，且宿主暂停时必须把发射器
   `speed_scale` 归零。
4. **弹龄与 `state_age` 的口径**：`age = frame_after_step − born_frame`。`advance`（相位 10）
   在 step 末尾 `+1`，故实体**第一次被画出来时 age = 1，不是 0**。
5. **高数量短寿命走实例缓冲 + shader，低数量长寿命走节点。** 这是当初"弹走 MultiMesh、
   自机走 Sprite2D"的同一条判据；本刀把敌人（cap 256、常态几十）划到节点侧（§5.1）。

## 3. 核（`stg-core`）

四项新增，全部过评审，`ENGINE_VER` **15 → 16**（布局 + syscall 号表两重变更）。

### 3.1 弹池 `born_frame: u32`

- 三条创建路径（`create_bullet_with_xform` 单发 / `create_bullets_batch` 批量 / shooter
  `sh_fire`）由 `define_pool!` 的 exhaustive `Init` 强制写入，值 = 创建时 `world.frame()`。
- 模拟相位不读它；只有编码器读。8192 × 4 B = 32 KB，不在热路径。
- 进校验和（P6，无例外）。

### 3.2 敌池 `anm_state_frame: u32`

- `spawn_enemy` 盖当前帧、`anm_state = 0`。
- `set_anm_state` **每次调用都重新盖帧**：同状态重设 = 重播（对应 ZUN interrupt 重触发语义）。
- 进校验和。

### 3.3 三个 syscall / 内建函数

| 内建 | 签名 | 语义 | 请求 |
|---|---|---|---|
| `set_anm_state` | `(state: Int)`，self-only | owner 敌 `anm_state = state as u16`，`anm_state_frame = frame`；owner 非 ENEMY → Fault（同 `move_to` 族） | — |
| `fx_at` | `(x: Fx, y: Fx, kind: Int, param: Int)` | 发请求 | `REQ_FX_AT = 8`，args `[x raw, y raw, kind, param, 0, 0]` |
| `fx_on` | `(kind: Int, param: Int)`，self-only | 发请求，句柄取 owner 敌；owner 非 ENEMY → Fault | `REQ_FX_ATTACHED = 9`，args `[index, gen, kind, param, 0, 0]` |

- 句柄传法对齐既有敌人**写**动词族（`move_to`/`set_*`：self-only，参数逆序弹出）；查询
  动词族（`enemy_hp` 等）的显式句柄形态不适用于写动作。
- `kind`/`param` 语义归内容包与壳侧约定，引擎不解释（与 sprite 号同待遇）。屏幕震动、
  闪白等全屏演出走 `fx_at` 的 kind 空间，不另开 id。
- 两条新请求都是**即发即忘**类。
- `consts.rs` ①段登记两个 id；`builtins.rs` 加三条；`gen-ecl-meta` 重跑同步
  `docs/ecl-lang/7-reference.md` 与 VS Code 元数据。

### 3.4 第四条纯输出缓冲 `vanished`

- **只记弹**。自机弹消失已有 `EVT_SHOT_HIT_ENEMY`，道具拾取已有 `EVT_ITEM_PICKED`。
- 行结构 `#[repr(C)]` 12 B：`x: Fx, y: Fx, sprite: u16, reason: u8, _pad: u8`。
- `reason` 两种：`VANISH_LIFE = 1`（`life` 归零）、`VANISH_CLEARED = 2`（`BULLET_CLEARED`
  位：作用区清除，含 bomb / deathbomb / 自机中弹清屏）。**越界不记**——屏外没有淡出可画，
  也免得白占行。
- cap **1024**（`VANISHED_CAP`），12 KB；bomb 峰值实测约 814 颗（自机能力刀），够用。
  超限确定性丢弃并计 `diag.vanished_overflow: u32`（**进校验和**，同 `events_overflow`）。
- 写入点：`world/cleanup.rs` 弹回收分支，`free_index` 前写行。时停冻结时 cleanup 早退，
  本帧自然无行。
- `begin` 清空，与 `frame_events` 同节奏：**必须在两次 step 之间取走**。
- 缓冲本身按 P6 标 `#[checksum(skip = "纯输出缓冲，帧首清空，与 reqs/hits/frame_events 同族")]`。
- 读口 `World::vanished(&self) -> &[Vanished]`。
- `copy_into` 同步；尺寸哨兵测试要**红一次再绿**。三个新字段都是 u32/整块数组，不会被
  D20 那个对齐 padding 盲区吞掉，但仍要实证哨兵真的响了。

### 3.5 回归口径

- 金向量 md5 **必然变**（两个新字段进校验和），收口时记新值进 `PROGRESS.md`。
- 变化只允许来自：新字段入哈希、`ENGINE_VER`。行为不得变——`vanished` 不改任何相位的
  决策，`born_frame`/`anm_state_frame` 无消费者。

## 4. 桥（`stg-godot`）

### 4.1 层号重排

敌层退役（拍板 ④甲案），不留空洞：`LAYER_BULLETS = 0, LAYER_SHOTS = 1, LAYER_ITEMS = 2,
LAYER_COUNT = 3`。GDScript 只认 `WorldBridge.LAYER_*` 符号；`playfield.gd` 的
`CAPS/CELLS/COLS/ROWS/TEXTURES/Z_ORDER` 镜像缩成三行。

### 4.2 弹层 custom.y = 弹龄

`frame − born_frame` 饱和到 65535 后转 f32；custom.z/w 仍为 0，stride 12 不变。其余层
custom.y/z/w 仍为 0。

### 4.3 弹旋转改查表

`bullet_basis` 改用 `stg_core::math::sincos(angle + 16384)` 取定点结果 `/ 65536`，不再调
libm `sin/cos`（满弹 8192 颗 = 每帧 1.6 万次三角函数）。补偿量 +16384 不变；既有判别式
单测"贴图的上 == 速度方向"与"朝上飞不旋转"原样守着。

### 4.4 新读口 `puppets()`

- 只装敌人。返回 Dictionary，每键一条压缩列，按池索引升序压实：
  `index, gen, sprite, anm_state, state_age, hit_flash` → `PackedInt32Array`；
  `x, y` → `PackedFloat32Array`。
- **无 `dying` 列**：敌人 dying 位在 settle 置、同一 step 的 cleanup 回收，step 后壳侧永远
  读不到 dying == 1；死亡动画数据源是 `REQ_ENEMY_DEATH`（带坐标与 sprite）。
- 编码逻辑放纯 Rust 模块 `crates/stg-godot/src/puppets.rs`（与 `frame.rs` 同款，不含
  godot 类型，输出 `Vec` 列），单测直接打列；`bridge.rs` 只做包装。
- 自机**不进**这里：继续走 `player_pos()` + `hud_player()`，后者补 `facing` 键。

### 4.5 新读口 `vanished()`

Dictionary：`x, y` → `PackedFloat32Array`，`sprite` → `PackedInt32Array`，
`reason` → `PackedByteArray`。

### 4.6 新读口 `entity_pos(kind, index, gen) -> Variant`

kind 目前只认敌人（0）；句柄有效返回 `Vector2`，失效返回 `null`。这是 `fx_on` 依附特效的
唯一跟随手段。

### 4.7 常量导出

`REQ_*`（含新 8/9）与 `EVT_*` 经 gdext `#[constant]` 从桥导出；`dispatcher.gd` 与
`main.gd` 的两份手抄镜像删除（理由同 `main.gd` 里 `RANK_*` 那条注释：手抄镜像与核无编译期
押运）。

### 4.8 上传路径

- 桥持有每层一个 `PackedFloat32Array`，编码器经 `as_mut_slice()` 直接写入（godot 0.5.4
  已有），省掉 `Vec` → Packed 那次拷贝；`multimesh_set_buffer` 内部那次拷贝无法省。
- `register_layer` 的长度判据（`== cap × 12`）不动。
- `custom_aabb`：壳侧 `playfield.gd` 建层时 `RenderingServer.multimesh_set_custom_aabb`
  设成场界矩形（含 OOB margin），绕过 Godot 对全部实例的 AABB 重算（godot-proposals #957
  指认的最大性能坑）。
- 前缀（undersized）上传**不做**：proposal 未落地，Godot 4.6 行为未证。

### 4.9 不动的

`take_requests`、`frame_events`、`anchors`、`hud_boss`、`hud_spell`、`save_state`/
`load_state`、`new_game`/`new_game_at`、`step_frame` 签名。请求分类与水位在壳侧分发器，
桥不掺和。

## 5. 壳（`godot/`）

### 5.1 敌人木偶池（`playfield.gd`）

- 启动时**按池索引预分配 256 个 `AnimatedSprite2D`**，数组下标 = 池索引，全部隐藏，永不
  释放（避免杂兵段每几帧生灭节点的分配抖动；也免去按句柄的字典）。
- 每帧读 `puppets()`：对出现的行，若节点记录的 `gen` ≠ 行 `gen` 视为新生、重置节点；
  然后设位置、按 `(sprite, anm_state)` 查动画名、按 `state_age` **直接设帧号**。
  **永远手动设帧，不用 AnimatedSprite2D 自动播放**（它走壁钟，违反 §2.3）。
  未出现在本帧的节点隐藏。
- 动画表放 `content_tables.gd`：sprite 号 → SpriteFrames 资源；状态号 → 动画名 + 每帧持续
  帧数；查不到退化为图集单格。约定一个状态号（`ANM_HIDDEN`）映射为隐藏，覆盖"隐形敌"需求。
- 受击闪白：每个木偶挂同一 shader 的 ShaderMaterial 副本，uniform `flash` 每帧由 `hit_flash`
  列设。256 份材质副本可接受。
- 节点序：木偶节点插在 items 层之后、bullets 层之前（保持原 Z 序）。

### 5.2 特效（`effects.gd` 重写）

- 按种类各一个 `GPUParticles2D` 发射器，起步三个：`explosion` / `hit_spark` / `bullet_fade`，
  全部 `emit_particle` 手动发射；节点式 `_Ring`/`_Spark` 退役。
- `bullet_fade` 数据源 = `vanished()`：sprite 号塞进粒子 custom，其 shader 选同一格弹图集
  做淡出，视觉上就是"那颗弹在消失"。
- `explosion` ← `REQ_ENEMY_DEATH`；`hit_spark` ← `EVT_SHOT_HIT_ENEMY`。
- 飘字仍是 Label 节点（数量小）。
- 宿主暂停：所有发射器 `speed_scale = 0`（§2.3）。
- Z 重开（`_boot(0)`）时清空发射器与依附池（顺手销 A9 ⑤ 里"重开残留"那半条）。

### 5.3 依附特效（`fx_on`）与定位特效（`fx_at`）

- 依附：小型跟随节点池，键 `(index, gen)`，每帧调 `entity_pos`，返回 `null` 即回收。
- `kind` → 场景/发射器映射表在 `content_tables.gd`。`fx_at` 按 kind 分到发射器或一次性场景。

### 5.4 分发器（`dispatcher.gd`）

- 分类表：`REQ_ENEMY_DEATH / REQ_FX_AT / REQ_FX_ATTACHED` 即发即忘；
  `REQ_SPELL_DECLARE / REQ_SPELL_RESULT / REQ_STAGE_CLEAR` 须确认；
  `REQ_BGM / REQ_BG / REQ_BG_PHASE` 电平镜像。
- `drain(arr: Array, confirmed_frame: int)` 带水位：`frame ≤ watermark` 的丢弃；须确认类
  缓冲到 `frame ≤ confirmed_frame` 才播；播完推进水位。
- **现在 `confirmed_frame == 当前帧`，行为与今天完全一致**；M3 接入回滚时只改这个实参。

### 5.5 主循环（`main.gd`）

- `_after_step` 顺序：分发请求 → 事件 → `vanished` → 木偶 → HUD → 自机。
- 冒烟：两条敌层断言（"杂兵真的在动"/"boss sprite == 1"）改读 `puppets()` 列——纯数据
  断言，不再依赖渲染器缓冲。

## 6. 契约文档（`docs/render-contract.md`）

- **§0 总规则**（新，放最前）：本文 §2 五条。
- **§2 实例布局**：弹层 custom.y = 弹龄；其余层 custom 为 0；层号三层。
- **§4 请求分发**：约定表加"类别"列，加 `REQ_FX_AT`/`REQ_FX_ATTACHED` 两行；写明水位语义。
  事件表每 kind 加"聚合口径"列（`EVT_SHOT_HIT_ENEMY` 逐条；`EVT_FIELD_CLEARED` 聚合；
  擦弹若将来事件化须先定口径——B28 原话）。
- **§3.6 `vanished`**（新）：行结构、reason 表、cap、取走时机。
- **§7 必须渲染项清单**（新）：每行 = 数据源 / 触发方式 / 大概样子 / 可否省略。起步：

| 项 | 数据源 | 触发 | 大概样子 | 可省略 |
|---|---|---|---|---|
| 弹层 | `LAYER_BULLETS` 缓冲 | 每帧 | 图集格 + 速度朝向旋转 | 否 |
| 自机弹层 | `LAYER_SHOTS` | 每帧 | 图集格，不旋转 | 否 |
| 道具层 | `LAYER_ITEMS` | 每帧 | 图集格 | 否 |
| 敌人木偶 | `puppets()` | 每帧 | 按 `(sprite, anm_state, state_age)` 选帧 | 否 |
| 自机 + 判定点 | `player_pos()`/`hud_player()` | 每帧 | 单图；`BTN_SLOW` 显判定点；`invuln` 闪烁 | 判定点否 |
| 弹出现闪光 | custom.y（弹龄） | shader | age 小时放大 + 提亮，数帧内收敛 | 是 |
| 消弹淡出 | `vanished()` | 每帧 | 同格贴图原地淡出 | 是 |
| 受击闪白 | `puppets().hit_flash` | 每帧 | 木偶叠白 | 是 |
| 敌死爆炸 | `REQ_ENEMY_DEATH` | 边沿 | 环/粒子 + 分数飘字 | 是 |
| 命中火花 | `EVT_SHOT_HIT_ENEMY` | 边沿 | 小火花 | 是 |
| 时停滤镜 | `freeze_left()` | 每帧 | 全屏色调 | 是 |
| HUD | `hud_*` | 每帧 | 数字 + boss 条 + 符卡行 | 否 |
| 横幅 | `REQ_SPELL_*`/`REQ_STAGE_CLEAR` | 边沿（须确认） | 文本 | 是 |
| 背景 | `anchors()` + `REQ_BG*` | 电平镜像 | 见 A4 | 是 |

  这张表既是换渲染器的抄写清单，也是有头目验的检查单。

- `reqs.rs` / `events.rs` 模块文档同步加列；`CLAUDE.md` 仓库结构段（`frame.rs` 描述行、
  `stg-godot` 行）随层号更新；`PROGRESS.md` 收口时史加一行 + 重写「现在」段。

## 7. 测试与验收

### 7.1 单测（TDD，先红后绿）

核：
- `born_frame`：三条创建路径各一条，断言 == 创建帧。
- `set_anm_state`：重设盖帧一条（同状态两次调用，第二次帧号更新）；owner 非 ENEMY → Fault 一条。
- `fx_at`/`fx_on`：请求 id 与 args 布局各一条；`fx_on` owner 非 ENEMY → Fault。
- `vanished`：LIFE 一条、CLEARED 一条、越界不记一条、溢出计数一条、`begin` 清空一条、
  时停冻结帧无行一条。
- 尺寸哨兵：红一次再绿（实证响了）。
- 金向量 md5 更新。

桥：
- `puppets.rs`：空池一条、压实序 + 列值一条（含 `state_age` 首帧 = 1）。
- `vanished` 透传一条。
- `entity_pos`：活句柄返 `Vector2`、死句柄返 `null` 各一条。
- 弹龄：N 步后 custom.y == N（注意首帧 = 1 口径）。
- `bullet_basis` 既有两条判别式单测不动。

### 7.2 验收顺序

1. `cargo test --workspace` 全绿；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all -- --check`。
2. `cargo tree -p stg-core` 防火墙不变。
3. `gen-ecl-meta` 重跑后 `docs/ecl-lang/7-reference.md` 与 `editors/vscode/.../ecl-meta.json` 无 diff 残留。
4. 桥级冒烟 `crates/stg-godot/smoke/run-smoke.sh` 绿（层号改）。
5. 真工程冒烟 `godot/smoke/run-smoke.sh` 绿（两条断言改读 `puppets()`）。
6. **VNC 有头目验**（走全局 CLAUDE.md 的 VNC 桌面，`godot --path godot` 非 headless）跑一局
   到风铃卡，肉眼确认：弹出现有闪光、bomb 消弹有淡出、杂兵中弹闪白、敌死有爆炸。这四件
   headless 一件都验不了；顺带销 B26 ②③。

### 7.3 实施顺序

核 → 桥 → 壳，**三段各自独立提交、各自跑绿**；中途任一段停下仓库都是一致的。核先行是
因为它 bump `ENGINE_VER` 且动金向量。

## 8. follow-ups 的销与记

- **F4** 部分销：`custom_aabb` + 免拷贝落地；前缀上传仍待 Godot 支持，条目改写为剩余部分。
- **B26** ②③ 随有头目验一并销。
- **B28** 擦弹聚合口径写进事件表；事件本身仍不发，条目保留。
- **A9** ⑤ 的"重开残留"半条销；③④ 顺手看能销几条，销不了不硬凑。
- 新记：`puppets()` 的 Dictionary-of-PackedArrays 每帧分配约九个数组——常态 <50 行无感，
  若将来 profiler 指认再改成桥持有复用。

## 9. 人类拍板记录

| # | 议题 | 裁定 |
|---|---|---|
| ① | 要不要做 ANM VM | **不做**。计时器归核、曲线归 shader、编排归 Godot 原生。 |
| ② | 粒子池进 Rust 还是留 Godot | **留 Godot**（我方曾推荐进 Rust，人类反问后撤回：Godot 有 GPU 粒子；headless 不关心粒子）。 |
| ③ | 立 `stg-present` 中立 crate | **不立**（曾拍板立，随后人类回退：契约写文档即可，换渲染器抄壳不费事）。 |
| ④ | 敌层退役 vs boss 节点杂兵 MultiMesh vs 全 MultiMesh | **甲：退役，全节点**，按池索引预分配 256 节点。 |
| ⑤ | 消弹淡出数据源：核内濒死几帧 vs 纯输出缓冲 | **B：`vanished` 输出缓冲**，模拟行为零改动。 |
| ⑥ | ECL 侧 ANM 互动指令 | 加三个：`set_anm_state`、`fx_at`、`fx_on`；`set_sprite` 运行期改口、alpha/scale/color 插值、layer/blend 一律不进核。 |
| ⑦ | 自机是否进 `puppets()` | 不进，走既有 `player_pos`/`hud_player`（补 `facing`）。 |
