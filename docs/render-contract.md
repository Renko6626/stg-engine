# 渲染契约（表现层权威；首要读者：美术 + Godot 壳作者）

## 1. 两条通道（定位）

通道 A 状态视图 → 四层实例缓冲（本文 §2-3）；通道 B 离散请求 → 分发器（§4）。
权威上游：`crates/stg-godot/src/frame.rs`（编码器）/ `crates/stg-core/src/reqs.rs`（请求 id）。

## 2. 实例缓冲布局（冻结，stride 12）

`[cos,-sin,0,x, sin,cos,0,y, sprite,0,0,0]` —— 前 8 = `MULTIMESH_TRANSFORM_2D`，
后 4 = `INSTANCE_CUSTOM`；`custom.x=sprite` 号，`y/z/w` 保留（将来 scale/alpha/调色，stride 不变）。
bullets 层带旋转（basis=角度），其余层单位 basis。压实前缀 + `set_visible_instances`。

## 3. 图集契约（每层独立 PNG + 独立 id 空间）

| 层 | 文件 | cell | 网格 | id 源 |
|---|---|---|---|---|
| bullets | assets/bullets.png | 32×32 | 8×1 | tables appearances[].sprite（现 0..3） |
| shots   | assets/shots.png   | 32×32 | 4×1 | shottype 表 sprite |
| enemies | assets/enemies.png | 64×64 | 4×1 | spawn_enemy sprite 参（A5 起脚本自给） |
| items   | assets/items.png   | 32×32 | 8×1 | tables item_cfg[].sprite |

sprite 号 = 格号（行优先）；越界号 mod 回卷。QuadMesh 尺寸 = cell 尺寸（1px=1unit）。
自机 assets/player.png（32×32 单图）/判定点 assets/hitbox.png（16×16）。
换真美术：只换 PNG（同网格），契约与代码零改动；要变网格，改本表 + playfield.gd 常量即可。

**sprite 号截断/回卷语义（跨层唯一权威）**：syscall 侧 `sprite` 参在入池前做
`sprite as u16` 截断——只取低 16 位，脚本传入的越界值（负数或 > 65535）在核心层就已经静默
折叠成某个 `u16`，池内 `sprite` 字段本身即为 `u16`（`enemy.rs`/`bullets.rs` 等同款）。核心层
**不**对 sprite 号做"是否落在本层图集网格内"的语义校验——那是表现层的职责边界。渲染侧拿到
这个 `u16` 后，按各层网格总格数（bullets/shots/items = 8，enemies = 4）取模回卷得到最终格号
（`cell_index = sprite % cols`），越界号因此总能落到某个合法格、绝不越界访问图集像素，但视觉
上会与低号格"撞车"（这是刻意的表现层容错，不是 bug）——脚本作者应把 sprite 号控制在网格范围
内，回卷只是兜底、不是可依赖的取号策略。

## 4. 请求分发（引擎保留段 1..=63，现分配 1..7）

表：`reqs.rs` 模块文档为准（id/args 逐位）；GDScript 侧 `dispatcher.gd` 本地常量镜像。
64+ 脚本段：内容包经 `dispatcher.register(id, callable)` 自注册。

## 5. 锚点双表示规矩（硬规矩）

事件（`REQ_BGM`/`BG`/`BG_PHASE`）= 边沿；`anchors()` 四字段 = 电平。宿主在 `new_game_at`/
`load_state` 成功后必须一次性读 `anchors()` 对表；游玩期只走请求增量。
bg 段内局部时间 = `frame - bg_phase_frame`（A4 mini-VM 的 seek 契约，本刀 phase 硬编码）。

## 6. 坐标与画面

场界 x∈[-192,192], y∈[0,448]（中轴原点）；SubViewport 384×448 @容器(32,16)，
世界根 Node2D@(192,0)；640×480 窗口，canvas_items 拉伸。定点→浮点只发生在消费端边界，
公式统一 `raw/65536`（编码器 `frame.rs::fx_f32`；`bridge.rs` 各读口如 `hud_player`/
`hud_boss` 的 `hp_ratio`/`player_pos`/`fields_info` 内联同式）。
