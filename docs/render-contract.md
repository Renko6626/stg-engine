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
| bullets | assets/bullets.png | 16×16 | 16×12 | tables appearances[].sprite（identity：id 即格号） |
| shots   | assets/shots.png   | 32×32 | 4×1 | shottype 表 sprite |
| enemies | assets/enemies.png | 64×64 | 4×1 | spawn_enemy sprite 参（A5 起脚本自给） |
| items   | assets/items.png   | 32×32 | 8×1 | tables item_cfg[].sprite |

sprite 号 = 格号（行优先）；越界号 mod 回卷。QuadMesh 尺寸 = cell 尺寸（1px=1unit）。
自机 assets/player.png（32×32 单图）/判定点 assets/hitbox.png（16×16）。
换真美术：只换 PNG（同网格），契约与代码零改动；要变网格，改本表 + playfield.gd 常量即可。

**UV 垂直朝向存疑，待 GPU 判决**（`docs/follow-ups.md` B23）：`layer.gdshader` 图集选格是否
上下镜像贴图尚未在真渲染器上验证（本机无 GPU/无 X）；占位图元已改为上下明暗渐变，有头
启动可一眼判别（判决程序见 follow-ups B23/B26）。换上下不对称的真美术前必须先跑 B23 的
判决程序。

占位图集"再生成对拍"验证（`gen_atlas.gd` 重跑、`md5sum` 比对生成产物与 commit 版本逐位相同）
需要本机装有真 Godot 二进制才能跑（`--headless --path godot --script res://tools/gen_atlas.gd`；
不是 GPU 问题，纯粹是 CI 镜像不装 Godot 可执行文件），CI 不可跑，只能本机手工核对。

**bullets 层的二维布局（颜色轴刀，2026-07-26）**：`sprite 号 = 弹型 × color_stride + 颜色`，
其中 `color_stride` 是 `WorldTables` 的字段（内建 = 16），**不是引擎常量**——mod 表可自定义
列数。表索引 ≡ 图集格号 ≡ 池 `sprite` 值（identity），故 `set_sprite` 与 `fire` 写的是同一个
数域。**当前内建图集（真美术，12 行全满 16 色）没有空格**；稀疏弹型（某行只做了部分颜色）
仍要占满一整行、用不到的列留成**空格**：表里 `valid = false`，创建时被拒（编译期报错 /
运行期 Fault），绝不会造出"有判定但看不见"的弹。摆位纪律：缺色留空、**不压紧**，
否则色号语义跨行不一致，色名会撒谎。
网格常量仍住 `playfield.gd`（进表是未来 mod 加载刀的事）。

**弹层图集的来源**：`godot/assets/bullets.png` 由 `godot/tools/slice_bullet_sheet.gd`
从弹片切出（不再由 `gen_atlas.gd` 生成占位——那个脚本已移除弹层，否则一跑就覆盖真美术）。
cell 取 16 是因为原作弹片本就是 16×16 网格，切割零留白；**若将来补入 32×32 的大玉一类，
整张图要改按 32 排、小图元居中留白（1:1 不缩放），并同步 `playfield.gd` 的 `CELLS[0]`。**

**sprite 号截断/回卷语义（跨层唯一权威）**：syscall 侧 `sprite` 参在入池前做
`sprite as u16` 截断——只取低 16 位，脚本传入的越界值（负数或 > 65535）在核心层就已经静默
折叠成某个 `u16`，池内 `sprite` 字段本身即为 `u16`（`enemy.rs`/`bullets.rs` 等同款）。核心层
**不**对 sprite 号做"是否落在本层图集网格内"的语义校验——那是表现层的职责边界。渲染侧拿到
这个 `u16` 后，按各层网格总格数（bullets = 192〔16×12，颜色轴刀起多行〕，shots = 4，
items = 8，enemies = 4）取模回卷得到最终格号（`layer.gdshader::vertex()`：
`s = sprite % (grid_cols*grid_rows)`，再拆 `col = s % grid_cols`/`row = s / grid_cols`
定位格子——单行层里 `grid_cols == 总格数`，两步模运算恰好重合，故此前的措辞
`cell_index = sprite % cols` 只对单行层成立；bullets 变多行后不再重合，写全两步才准确），
越界号因此总能落到某个合法格、绝不越界访问图集像素，但视觉
上会与低号格"撞车"（这是刻意的表现层容错，不是 bug）——脚本作者应把 sprite 号控制在网格范围
内，回卷只是兜底、不是可依赖的取号策略。

## 4. 请求分发（引擎保留段 1..=63，现分配 1..=7）

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
`hud_boss` 的 `hp_ratio`/`player_pos`/`fields_info` 内联同式；第五处见 `main.gd`
`REQ_ENEMY_DEATH` 处理器——`_wire_requests` 里 `a[0] / 65536.0, a[1] / 65536.0` 把请求
携带的原始定点坐标换回浮点世界坐标喂 `effects.explosion`，同式内联在 GDScript 侧）。
