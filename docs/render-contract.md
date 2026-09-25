# 渲染契约（表现层权威；首要读者：美术 + Godot 壳作者）

## 0. 总规则（表现契约 v2，2026-09-07；spec `docs/superpowers/specs/2026-09-07-presentation-contract-v2-design.md`）

1. **电平走通道 A，边沿走通道 B。** 边沿可丢（cap 满确定性丢弃）可重放（回滚重演），
   **电平不可丢**。持续超过一帧、且真值在核里的表现状态，一律每帧从 `WorldView`/桥面读口
   拉电平（自机无敌闪烁读 `hud_player().invuln`、受击闪白读 `puppets().hit_flash`、
   时停滤镜读 `freeze_left()`），绝不用边沿事件在壳侧"记住"它。
2. **边沿只有三类用途**（分发器按类处置，§4）：即发即忘 / 须确认 / 电平镜像。
3. **表现层唯一时间源是 step 结束后的 `frame`。** 木偶动画位置、shader 里的 t、特效寿命全部由
   帧号推导；壁钟只允许飘字这种即发即忘的小件用。宿主暂停 = 帧不走 = 一切特效原地停。
4. **弹龄与 `state_age` 口径**：`age = frame_after_step − born_frame`。`advance`（相位 10）在
   step 末尾 `+1`，故实体**第一次被画出来时 age = 1，不是 0**。
5. **高数量短寿命走实例缓冲 + shader，低数量长寿命走节点。** 弹 / 自机弹 / 道具 / 特效走
   MultiMesh；敌人（cap 256、常态几十）与自机走节点。
6. **核不跑任何动画，只出「三件套」事实**（§0.5）：`visual` + `phase` + `phase_frame`。
   不设通用表现参数槽，不做 ANM VM，不做 ANM 语言，不立中立表现 crate。

## 0.5 表现数据模型与生命周期纪律（2026-09-11 立规）

> 背景：一份外部建议把 ZUN 的 ANM 生命周期拆成「嵌入式 / 句柄持有 / 放生式 / 混合态」，
> 并主张显式化为 `bound` / `oneshot` 两种脚本类型由编译器检查。本仓的答案是**同一套思想、
> 更少的机器**：那两种类型在这里是两条通道，"编译器验证"在这里是"表里就是常量"。
> 逐条对照见本节末表。

### 三件套：任何跨帧动画实体，核只提供三个字段

| 字段 | 含义 | 谁写 | 壳怎么用 |
|---|---|---|---|
| `visual` | 不透明的格号 / id（弹 `sprite` = 图集格号，敌 `sprite` = 脚本原样传入） | 脚本 / 表 | 选图集、选节点纹理 |
| `phase` | **语义**状态（敌 `anm_state`、背景 `bg_phase`、自机 `life_state`） | 脚本，或核的语义逻辑 | 选 label / 动画段 |
| `phase_frame` | 写 `phase` 那一刻的帧号（敌 `anm_state_frame`、背景 `bg_phase_frame`） | 与 `phase` 同一笔写 | `age = frame − phase_frame` 选帧、算曲线 |

- **ZUN 的 interrupt 在这里是状态字段，不是事件。** 壳每帧对照 `phase` 变了就跳 label；因为
  是电平，回滚 / 遡行 / 读档后自动收敛，事件做不到。
- **其余驱动量必须是语义字段**：`hit_flash`（核递减）、`hp/hp_max`、`invuln`、`facing`、
  `freeze_left`、`boss_ui.hp_ratio`。**不设通用 `params[4]`**——一个"sim 不读但进校验和"的
  槽是垃圾位的温床，且在回滚下与语义字段没有任何区别。脚本要给壳传表现参数：一次性走
  `fx_at(kind, param)` / `fx_on`，持续的加一个**有名字**的语义字段。
- **存储是 SoA 分散的，抽象是统一的。** 各池按 I7 各自持有自己的三件套（弹没有 `phase`，
  它的画面是 `(sprite, age, angle)` 的纯函数）；统一的是**规矩与读口形状**，不是一个嵌套
  struct。当第三类带 `phase` 的实体（自机动画帧）落地时，顺手把命名对齐成 `*_phase` /
  `*_phase_frame` 并在 `WorldView` 上给同形状读口（follow-ups F23）。

### 全景（各类实体现状）

| 实体 | visual | phase | phase_frame | 其他驱动量 | 壳侧渲染 |
|---|---|---|---|---|---|
| 弹 | `sprite` | 无 | `born_frame` | `angle`、`flags` | MultiMesh + shader |
| 自机弹 | `sprite` | 无 | 无 | 无 | MultiMesh |
| 道具 | `sprite`（`item_type` 查表） | 无 | 无 | 无 | MultiMesh |
| 敌 | `sprite` | `anm_state`（只脚本写） | `anm_state_frame` | `hit_flash`、`hp`、`invuln` | 256 个预分配 Sprite2D 木偶 |
| 自机 | **无**（F23） | `life_state` | `state_timer` | `invuln`、`facing`、`jump_cd`、`deaths` | 单张 Sprite2D |
| 背景 | `bg_id` | `bg_phase` | `bg_phase_frame` | 无 | Bg 节点（A4 mini-VM 未做） |
| 特效 | 不在核 | 不在核 | 壳记 `born` | 请求载荷 `kind/param` | fx MultiMesh 池（§4） |

### 生命周期：两条通道就是两种所有权

| ZUN 的说法 | 这里 | 谁负责销毁 |
|---|---|---|
| 嵌入式（弹内嵌 VM） | 弹没有 VM，画面是快照的纯函数 | 池回收即消失；淡出走 `vanished`（§3.6） |
| 句柄持有（敌 / boss） | 木偶节点按池索引预分配，`(index, gen)` 识别新生 | 实体出池 → 木偶隐藏；退场动画走 oneshot（F21） |
| 放生式（火花） | fx 行 `(kind, born, param)`，寿命 = `FX_LIFE` 表常量 | 壳按 `age ≥ life` 回收；**有限长是表的性质**，不需要编译器验证 |
| 混合态 / detach | `EVT_ENEMY_DIED{a_index, a_gen}` 已带代数；木偶转残影 oneshot 即 detach | 壳侧，数据齐全（F21） |
| 子 VM 跟随父级 | `fx_on` 依附行每帧 `entity_pos(index, gen)`，句柄失效即回收 | 父灭子灭；没有孤儿，因为没有 detach 的子级 |

### 时间跳变（遡行 / 跳躍快进 / 读档 / 回滚）下的表现规则

1. 电平（通道 A）自愈：重画即正确，无需任何特判。
2. 边沿（通道 B）按分发器水位去重（§4）：落点 F 及以前的请求已呈现过，不重播。
3. oneshot 清理**按出生帧**：杀 `born > F` 的行，`born ≤ F` 的继续活（落点前开始的爆炸环
   不该被误清）。现实现是 `clear_all()`，改按出生帧记 F20。
4. 预测回滚（M4）才有的三分支——"有 release 事件 / 没有就消失 / 残影又复活"——单机不存在：
   遡行是整段丢弃，落点在死亡之前，敌人就在快照里，木偶按 gen 记忆直接复用。留到 `stg-net`。

### 核需要动画时长时

「等施法动画播完再开火」这类需求**不许**让核执行动画：时长在**编译期**成为常量。做法是把
`ENEMY_ANIM` / `FX_LIFE` 迁进内容数据文件，照 `gen-ecl-meta` 双生成——ECL 常量一份
（`wait(ANM_LEN_BOSS_CAST)`）、GDScript 表一份（F22）。反方向的规则更硬：**凡影响判定的量
一律在核**（激光宽度、boss 判定位置），ANM 只负责画。

### 四件明确不做的（历次拍板汇总）

| 不做 | 拍板 | 理由 |
|---|---|---|
| ANM VM | v2 ① | 计时器归核、曲线归 shader、编排归 Godot 原生 |
| 自造 ANM 脚本语言 + 编译器（`bound`/`oneshot` 声明类型） | 本节 | 声明类型已被"两条通道 + 表驱动 + 帧号时间源"消解；真要脚本化演出，先考虑 Godot AnimationPlayer 资源 |
| 独立 Rust ANM runtime crate 直调 RenderingServer | v2 ③（立了又撤） | 没有 VM 就没有 runtime；桥已是"Rust 编码缓冲、一次上传"，壳只剩薄胶水 |
| 每 VM / 每实例一个 Node | v2 ④ | 敌人 256 节点是**预分配常驻**不是逐实例创建；弹 / 特效走 MultiMesh |

## 1. 两条通道（定位）

通道 A 状态视图 → 四层实例缓冲（本文 §2-3 + §3.10）+ 敌人木偶喂料（§3.7）+ 事件流（§3.5）+
`vanished`（§3.6）；通道 B 离散请求 → 分发器（§4）。
权威上游：`crates/stg-godot/src/frame.rs`（编码器）/ `crates/stg-godot/src/puppets.rs`（木偶）
/ `crates/stg-core/src/reqs.rs`（请求 id 与类别）/ `crates/stg-core/src/events.rs`（事件与聚合口径）。

## 2. 实例缓冲布局（冻结，stride 12）

`[cos,-sin,0,x, sin,cos,0,y, sprite,age,0,0]` —— 前 8 = `MULTIMESH_TRANSFORM_2D`，
后 4 = `INSTANCE_CUSTOM`；`custom.x=sprite` 号，**`custom.y = 弹龄`（弹层有语义，其余层恒 0；
激光层例外见下；§0 口径，首帧 1）**，`z/w` 保留（将来 scale/alpha/调色，stride 不变）。
bullets 层带旋转，其余三层单位 basis。压实前缀 + `set_visible_instances`。
层号：`LAYER_BULLETS=0 / LAYER_SHOTS=1 / LAYER_ITEMS=2 / LAYER_LASERS=3`（**敌层已退役**，
敌人走 §3.7 木偶；`LAYER_COUNT=4`）。
**激光层是唯一的例外：基自带缩放、`custom.y = alpha`**（不走图集，见 §3.10），
其余三层照上式。

**shader 不得乘片元 `COLOR`（有头目验判决，2026-09-07）**：MultiMesh 未开 `use_colors` 时
片元 `COLOR` 输入是未定义的逐像素垃圾（GL/Vulkan 两后端一致的彩色噪点），乘上去弹就成了
碎彩点。这是场景刀以来一直存在的 bug，16px 的弹看着像"彩色小球"，B23 那次目验没辨出来。
`layer.gdshader`/`fx.gdshader` 现都是 `COLOR = tex`；整层调色/淡出走 uniform 或 custom.z/w。

**弹层出现闪光**：`layer.gdshader` 的 `spawn_flash_frames` uniform（弹层设 6，其余层 0 = 关）
按 `custom.y` 在前几帧放大 + 提亮，纯 shader、CPU 零成本。

**上传路径**：桥持每层一个 `PackedFloat32Array`，编码器 `as_mut_slice` 原地写；壳侧
`multimesh_set_custom_aabb` 钉成场界矩形，绕过 Godot 对全部实例重算 AABB（godot-proposals
#957）。前缀（undersized）上传未做（Godot 未支持），账见 follow-ups F4。

**弹的朝向约定（硬规矩）**：渲染旋转 = **速度方向 + 四分之一圈**（`frame.rs::bullet_basis`）。
两个基准差 90°：世界侧 `polar_to_vec = (speed·cos, speed·sin)` 所以 **BAM 0 指 +x（右）**，
而图集里的弹**画的是头朝上**（原作弹片惯例）。补 +16384 之后，朝上飞的弹（BAM 49152）
**不旋转**、正好头朝上；BAM 0（朝右飞）转 90°（屏幕 y 向下，正角即顺时针）→ 头朝右。
等价表述、也是单测钉住的不变量：**贴图的"上"经实例变换后等于速度方向**。
**换真美术时若图元不是头朝上画的，改这里的补偿量，不要改 shader。**

## 3. 图集契约（每层独立 PNG + 独立 id 空间）

| 层 | 文件 | cell | 网格 | id 源 |
|---|---|---|---|---|
| bullets | assets/bullets.png | 16×16 | 16×12 | tables appearances[].sprite（identity：id 即格号） |
| shots   | assets/shots.png   | 32×32 | 4×1 | shottype 表 sprite |
| items   | assets/items.png   | 32×32 | 8×1 | tables item_cfg[].sprite |
| lasers  | **无图集**（程序化截面，§3.10） | — | — | 池 `sprite` = 颜色号 0..15 |
| （木偶）enemies | assets/enemies.png | 64×64 | 4×1 | spawn_enemy sprite 参（A5 起脚本自给）——**不是 MultiMesh 层**，Sprite2D `hframes=4` 选格，见 §3.7 |

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

## 3.5 事件流 `frame_events()`（通道 A 的批量事实出口）

桥面读口，返回本帧的世界大事记（`stg_core::events::EVT_*`）：每条带 `kind` /
`x`,`y`（世界坐标，已转浮点）/ `a_index`,`a_gen`（相关实体句柄）/ `data0`,`data1`
（逐 kind 约定见 `events.rs`）。**帧内缓冲，下一次 `step` 的 `begin` 清空——必须在两次
step 之间取走。**

**与 §4 请求的分工（别混）**：请求是脚本/引擎主动发的**离散演出指令**（宣言、挂牌、BGM
切换）；事件是世界每帧产出的**事实流**（敌死 / 自机死 / 消弹 / 拾取 / 符卡宣言收卡失败 /
任务 fault / 自机弹命中）。表现层要"跟着世界发生的事做反应"（火花、音效、伤害数字）走事件。

`EVT_SHOT_HIT_ENEMY`（自机弹命中敌，2026-07-27）：**`x`/`y` 是自机弹的位置，不是敌心**
——命中点在弹上。逐命中发不聚合（自机弹同屏受池 cap 1024 限，现实 <50/帧，`EVENTS_CAP=512`
够用）；对照 `EVT_FIELD_CLEARED` 必须聚合（弹池 8192 远超 events 512）。
**擦弹若将来也要发事件，频率高一个量级，须单独评估聚合口径。**

`EVT_REWIND_REQUESTED`（时间机制内核刀，2026-09-07）：自机在决死窗口内按了遡行，
`a_index` = 自机号、`data0` = 进入决死窗口那一帧的帧号。**世界只发请求不改状态**，兑现
（恢复快照 + 落地写）归 `stg_core::timeline`；桥面把它消化成 `step_frame` 的返回值（落点帧），
壳侧**不该**从事件流里再处理它一遍——它在 `frame_events()` 里出现的那一帧已经被回滚掉了。

## 3.6 `vanished`（本帧离开池的敌弹；表现契约 v2）

核内第四条纯输出缓冲（与 `reqs`/`hits`/`frame_events` 同族：帧内私有、checksum-skip、`begin`
清空、回滚重演确定性再生）。桥面 `vanished()` 返 Dictionary：`x,y`（浮点世界坐标）、`sprite`
（弹图集格号）、`reason`（`VANISH_LIFE=1` 寿尽 / `VANISH_CLEARED=2` 被作用区清除，含 bomb、
deathbomb、`clear_bullets`）。**只记场内、越界不记**（屏外没有淡出可画）；同帧既越界又被清按
越界处置；被清优先于寿尽。cap 1024（bomb 峰值实测约 814），超限丢弃 + `diag.vanished_overflow`。停止冻结期间（玩法刀）只出现 `VANISH_CLEARED`：触碰消弹当帧回收；寿尽/越界留到解冻。
**必须在两次 step 之间取走。** 消费者：`effects.gd::fade_batch` 在原位用同一格弹贴图淡出。

## 3.7 敌人木偶喂料 `puppets()`（表现契约 v2）

敌层退役后通道 A 对敌人的唯一出口。返回 Dictionary，每键一条压缩列（按池索引升序）：
`index, gen, sprite, anm_state, state_age, hit_flash`（`PackedInt32Array`）、`x, y`
（`PackedFloat32Array`）。**无 `dying` 列**（敌人 dying 位在 settle 置、同一 step 的 cleanup
回收，step 后读不到；死亡动画走 `REQ_ENEMY_DEATH`）。壳侧（`playfield.gd`）按池索引预分配
256 个 Sprite2D，`gen` 变了 = 新生重置；`(sprite, anm_state, state_age)` 查
`content_tables.gd::ENEMY_ANIM` 手动选格（**永远不用自动播放**，§0 第 3 条）；约定状态号
`ANM_HIDDEN=255` = 隐藏；`hit_flash` 经 `puppet.gdshader` 的 `flash` uniform 叠白。
ECL 侧 `set_anm_state(n)` 写状态并盖帧（同状态重设 = 重播，即 ZUN `anmInterrupt` 的电平版）。
按句柄跟随（依附特效）走 `entity_pos(kind, index, gen)`：句柄失效返 `null`。

## 4. 请求分发（引擎保留段 1..=63，现分配 1..=9）

表：`reqs.rs` 模块文档为准（id/args 逐位 + **类别**）；GDScript 侧常量取 `WorldBridge.REQ_*`
（桥面导出，不手抄）。64+ 脚本段：内容包经 `dispatcher.register(id, callable)` 自注册，默认
即发即忘，可 `set_class` 改类。

**三类与水位（`dispatcher.gd`）**：

| 类别 | id | 处置 |
|---|---|---|
| 即发即忘 | `REQ_ENEMY_DEATH` / `REQ_FX_AT` / `REQ_FX_ATTACHED` | 收到即播；回滚误播接受为鬼影 |
| 须确认 | `REQ_SPELL_DECLARE` / `REQ_SPELL_RESULT`（`REQ_STAGE_CLEAR` 已退役，流程走 `EVT_STAGE_CLEARED` 事件） | 缓冲到 `frame ≤ confirmed_frame` 才播（M3 前 `confirmed_frame` = 当前帧，行为与从前一致） |
| 电平镜像 | `REQ_BGM` / `REQ_BG` / `REQ_BG_PHASE` | 边沿通知；真值在 `anchors()`，开机/读档后先对表（§5） |

水位：`frame ≤ watermark` 的请求丢弃（回滚重演已呈现过的帧）；重开/读档 `dispatcher.reset()`。

**一次性演出两条**（ECL `fx_at(x, y, kind, param)` / `fx_on(kind, param)`，对应 ZUN
`anmPlayPos` / `anmPlay`）：`kind`/`param` 引擎不解释，壳侧 `content_tables.gd` 的 `FX_*`
段定义；`fx_on` 的载荷是裸 `index, gen` 两位，壳侧起跟随行、每帧 `entity_pos`，失效即回收。

**事件聚合口径**见 `events.rs` 模块文档表；擦弹若将来事件化须先定口径（follow-ups B28）。

## 3.8 影子层（観測；时间机制内核刀 2026-09-07）

`preview(n)`：桥让 `Timeline` 的影子世界从权威世界克隆、喂一帧 `BTN_JUMP`、再走 `n` 帧
（`n == JUMP_FRAMES` 时影子就是跳躍**落地那一帧**），把影子的**弹层**按 §2 同一布局
（stride 12）编码上传到 `register_ghost_layer` 注册的 MultiMesh。**影子不碰权威世界、环、
输入日志**，是纯表现。壳侧 `ghost.gdshader` 按策划案 7.4「预测永远比实体暗、比实体细」
去饱和 + 压暗 + 半透明，压在实弹**之下**。v1 只有弹层，敌人木偶的未来态不做（follow-ups）。

## 3.9 倒放视图（`view_ring`；时间机制内核刀）

`view_ring(f)` 把快照环里第 `f` 帧（含**刚被遡行丢弃的分支**——下一次 `step_frame` 之前
那些槽原封不动）编码上传各层，并把**全部通道 A 读口**（层缓冲、`puppets()`、`hud_*`、
`anchors()`、`player_pos()`、`fields_info()`）切到那一帧；`vanished()` 在视图态恒空；
`frame()`/`checksum()`/`take_requests()`/`frame_events()` 始终是权威世界。下一次
`step_frame` 自动切回。宿主用它做「逐帧倒退」动画，不需要自己存任何历史。

## 3.10 激光层（激光池刀 2026-09-25；spec `2026-09-25-laser-pool-design` §7）

`LAYER_LASERS=3`（`LAYER_COUNT=4`），cap 256，编码器 `frame.rs::encode_layer` 逐条从
`view().lasers()` 读（池索引升序）。**不走图集**：截面渐变由 `godot/shaders/laser.gdshader`
程序化生成，`playfield.gd::_make_laser_layer` 的 QuadMesh 取 1×1，缩放全在实例基里，
stride 12 不变。

实例布局（`[xx, yx, 0, ox, xy, yy, 0, oy, custom.x, custom.y, 0, 0]`）：

| 分量 | 语义 |
|---|---|
| 局部 x 轴 `(xx, xy)` | 横截面方向（激光方向转 90°）× **显示宽度**（shader 的 `UV.x` 即截面） |
| 局部 y 轴 `(yx, yy)` | 激光方向 `(cos,sin)`（BAM 0 指 +x）× **可见长度 `end − start`** |
| 原点 `(ox, oy)` | 线段中点 = 射线原点 + dir × (start + end)/2 |
| `custom.x` | 颜色号 0..15（池 `sprite`）；shader 查 16 色表，索引序照 `bullets.ecl` 的颜色列 |
| `custom.y` | alpha（其余层 = 弹龄 / 0） |
| `custom.z/w` | 保留 |

**显示宽度 = 判定宽度（硬规矩，裁定 ④）**：核只存一个 `width`，判定半高 = `width/2`（D8 行 9）、
画面宽度也 = `width`；渐变的暗边也在判定内。TH06 原作画面是判定的 2 倍，这个口径差**由转写方**
写 `width = 原作值/2` 抹平，引擎不内置任何一作的口径。

**三态画面全在表现层算**（`frame.rs::laser_display`，核里不存画面状态）：预警 state 0 =
1.2 px 细线、最后 `min(warn, 30)` 帧线性长到全宽；生效 state 1 = 全宽；收缩 state 2 =
`flags` 位 0 为 1 时 alpha 线性到 0、否则宽度线性到 0。alpha 乘 `custom.y`，加色混合
（`laser.gdshader` 的 `render_mode blend_add`）。
原点闪光（原作 `SPAWN_BIG_BALL`）本刀不做，记 follow-ups。

> **被清弹取消的激光多一帧、且收缩首帧 `timer == 0`**（2026-09-25 控制方补充）：
> 清弹 field 在相位 7 把激光置 `state 2`、`timer = 0`（D8 行 10），此后相位 5 才会加
> `timer`；而自然到期 / ECL `lz_cancel` 的收缩首帧 `timer` 已是 1。于是取消的激光**收缩期
> 比自然到期或 ECL 取消多 1 帧**，且**收缩首帧 `timer == 0`**。渲染按 `timer / fade` 插值时必须容忍
> 这一点：`timer == 0` 当满宽/满 alpha 处理；`fade == 0` 时 `k = 0`（不能除零）。
> `laser_display` 已如此实现。

## 5. 锚点双表示规矩（硬规矩）

事件（`REQ_BGM`/`BG`/`BG_PHASE`）= 边沿；`anchors()` 四字段 = 电平。宿主在 `new_game_at`/
`load_state` 成功后必须一次性读 `anchors()` 对表；游玩期只走请求增量。
bg 段内局部时间 = `frame - bg_phase_frame`（A4 mini-VM 的 seek 契约，本刀 phase 硬编码）。

## 5.5 时停的表现层读口（`freeze_left`）

`WorldView::freeze_left()`（`world/view.rs`）暴露 `[u16; 2]`——`[0]` 是自机能力档（自机
按键触发的时停技能写它），`[1]` 是 ECL 演出档 `time_stop_player()`。两个槽都非零即代表
对应冻结组当前生效，数值是剩余帧数；表现层可以直接拿它画停时特效（比如给场景整体叠
一层滤镜、把 HUD 边框变色）而不需要另外猜测"现在是不是冻着"。

**bomb 不写这两个槽**——它的无敌走独立的 `invuln` 帧计时（`PlayerState.invuln`，
`hud_player()` 的 `invuln` 字段），`freeze_left` 全程不动。这正是 spec §8 要求的：bomb
期间相位 6/7 照常运行（判定与结算不停），只是自机有无敌帧、场上弹被消弹区清空——与
时停"相位 6/7 一起冻住"是两回事。把停时画面滤镜键到 bomb 上会什么都看不到。

**背景相位锚点在冻 C 时同步推进，表现层不需要自己特判**：`bg_phase_frame` 不是独立计时器，
是背景 mini-VM 的锚点——冻结期间它跟 `frame` 一起走，`frame − bg_phase_frame`（背景段内
局部时间）因此在时停全程保持不变，效果是"背景画面看起来也停住了"（bomb 期间背景照常
推进，因为 `freeze_left` 未变、这条豁免根本没生效）。这层豁免逻辑在核内，表现层只管
照常按 `anchors()`/请求增量算 `frame − bg_phase_frame`，不用为冻结状态另写一套背景
寻位分支。

## 5.6 遡行落地的表现规矩（时间机制内核刀 2026-09-07；spec §5）

`step_frame` 返回 ≥ 0 = 本 tick 发生了遡行，值是落点帧 F，**世界已经在 F**（恢复 + 落地写
`invuln`）。壳侧进入倒放表现态：从请求帧 G 起每 tick `view_ring` 往回读若干帧到 F，期间
不 step、不收输入。到 F 后三件事**必须做**：

1. 分发器水位 `reset_to(F)`——F 及以前的请求都已呈现过；须确认类的待播队列整个作废
   （它们属于被丢弃的分支）。
2. fx 池整个清空——落地后出生的行都不该活着；帧龄为负的行本来就该隐藏。
3. `_sync_anchors()`——遡行落地就是一次读档（follow-ups A7 记的那个场合），HUD 与背景电平
   要拉回 F。

跳躍的快进（缺席的 N 帧在同一 tick 走完）**丢弃**中间帧的通道 B 请求与 `vanished`：她不在
场，「线框态瞬时对齐为实体」。要让落地时补一次爆炸得让 timeline 攒事件（follow-ups）。

`EVT_REWIND_REQUESTED` 在 `frame_events()` 里出现的那一帧已经被回滚掉了，壳侧不该再处理。

## 6. 坐标与画面

场界 x∈[-192,192], y∈[0,448]（中轴原点）；SubViewport 384×448 @容器(32,16)，
世界根 Node2D@(192,0)；640×480 窗口，canvas_items 拉伸。定点→浮点只发生在消费端边界，
公式统一 `raw/65536`（编码器 `frame.rs::fx_f32`；`bridge.rs` 各读口如 `hud_player`/
`hud_boss` 的 `hp_ratio`/`player_pos`/`fields_info`/`puppets`/`vanished`/`entity_pos` 内联同式；
第五处见 `main.gd` `_wire_requests`——`REQ_ENEMY_DEATH`/`REQ_FX_AT` 处理器里
`a[0] / 65536.0, a[1] / 65536.0` 把请求携带的原始定点坐标换回浮点世界坐标，同式内联在 GDScript 侧）。

## 7. 必须渲染项清单（换渲染器的抄写清单，也是有头目验的检查单）

| 项 | 数据源 | 触发 | 大概样子 | 可省略 |
|---|---|---|---|---|
| 弹层 | `LAYER_BULLETS` 缓冲 | 每帧 | 图集格 + 速度朝向旋转（§2） | 否 |
| 自机弹层 | `LAYER_SHOTS` | 每帧 | 图集格，不旋转 | 否 |
| 道具层 | `LAYER_ITEMS` | 每帧 | 图集格 | 否 |
| 激光层 | `LAYER_LASERS` | 每帧 | 程序化截面（白芯 → 实例色渐暗，加色），预警/收缩在表现层算（§3.10） | 否 |
| 敌人木偶 | `puppets()` | 每帧 | 按 `(sprite, anm_state, state_age)` 选帧（§3.7） | 否 |
| 自机 + 判定点 | `player_pos()`/`hud_player()` | 每帧 | 单图；`BTN_SLOW` 显判定点；`invuln` 按帧号奇偶闪 | 判定点否 |
| 弹出现闪光 | `custom.y`（弹龄） | shader | age 小时放大 + 提亮，6 帧内收敛 | 是 |
| 消弹淡出 | `vanished()` | 每帧 | 同格弹贴图原地淡出（`fx` kind 0） | 是 |
| 受击闪白 | `puppets().hit_flash` | 每帧 | 木偶叠白 | 是 |
| 敌死爆炸 | `REQ_ENEMY_DEATH` | 边沿（即发即忘） | 橙色扩张环 + 分数飘字 | 是 |
| 命中火花 | `EVT_SHOT_HIT_ENEMY` | 边沿 | 小圆盘 7 帧 | 是 |
| 脚本演出 | `REQ_FX_AT` / `REQ_FX_ATTACHED` | 边沿 | 按 kind：闪点 / 依附光环 / 内容包自定 | 是 |
| 时停滤镜 | `freeze_left()` | 每帧 | 全屏色调 | 是（未接） |
| 影子层 | `preview(n)` → 影子弹层缓冲 | 観測期间每帧 | 去饱和压暗半透明的弹（§3.8） | 否（策划案核心机制） |
| 遡行倒放 | `view_ring(f)` | 落地后每 tick | 四层 + 木偶按环里那一帧重画（§3.9/§5.6） | 否 |
| 时间提示 | `hud_player().life_state` + 壳状态机 | 每帧 | 一行文字：観測 N / 跳躍 / V 遡行 | 是 |
| HUD | `hud_*` | 每帧 | 数字 + boss 条 + 符卡行 | 否 |
| 横幅 | `REQ_SPELL_*` / `EVT_STAGE_CLEARED`（事件） | 边沿（须确认 / 事实） | 文本 | 是 |
| 背景 | `anchors()` + `REQ_BG*` | 电平镜像 | 见 follow-ups A4 | 是 |

## 8. 有头目验（`--shots` 模式）

本机无 GPU 但有 VNC 桌面 + llvmpipe（全局 CLAUDE.md）。`main.gd` 的 `--shots` 模式用脚本化
输入（常按射击、60–75 帧向左、200 帧放 bomb、300 帧按 C 観測、330 帧再按 C 跳躍）跑 demo，
在若干帧把 SubViewport 存 PNG，并在首次敌死后第 4 帧补一张、遡行落地后补一张（若发生）；
同时打印各层（含影子层）`visible_instances` 真值。

```bash
cargo build -p stg-godot
DISPLAY=:2 LIBGL_ALWAYS_SOFTWARE=1 MESA_LOADER_DRIVER_OVERRIDE=llvmpipe STG_SHOTS_DIR=/tmp/shots \
  godot --rendering-driver opengl3 --path godot -- --shots
```

2026-09-07 判读（截图逐张放大核对）：弹为干净的图集格；受击敌人叠白 + 命中火花；bomb 后弹层
归零、`fx` 层用同格贴图淡出（淡出下面那块品红方块是星星道具的占位图，不是 bug）；敌死有
橙色爆炸环与飘字；`visible_instances` 弹层 4–15、自机弹层 9–10 为真值。存档帧号比目标帧
晚 1–4 帧（`await frame_post_draw` 之后才读 `frame()`），只影响文件名。

2026-09-07 追加判读（时间机制内核刀）：`observe_f325`——敌人下方无实弹处出现 5 颗去饱和的
暗色影子（`ghost` 层 `visible_instances=5`，实弹层 0）；`jump_f363`——跳躍落地后实弹层 6 颗
青色弹正好落在影子所在处，影子层已关。脚本化输入没撞上弹，故本次无 `rewind_land` 张；
遡行的数值判别在桥级冒烟（走进弹流 → 决死窗口 → 落点 = 被弹帧 − 30、`view_ring` 往返）。
