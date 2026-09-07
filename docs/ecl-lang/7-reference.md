# 7 · 速查：内建函数 / `$` 变量 / 引擎常量

> 这一篇是纯查表页，不讲道理——每一条的"为什么"在前六篇里。写脚本时把它开在旁边。
> 内建函数与 `$` 变量两段都是**从 `builtins.rs` 生成的**（`all()` / `ENGINE_VARS`），
> 改签名或加变量请改代码再跑 `gen-ecl-meta`。

## `$` 引擎变量（只读；读取即 syscall）

> 下表**从 `builtins.rs::ENGINE_VARS` 生成**（同一张表还喂编辑器的补全与 hover）——加变量
> 请改代码再跑 `gen-ecl-meta`，别手改这里。

<!-- gen:engvars:begin -->
| 名字 | 类型 | 含义 |
|---|---|---|
| `$frame` | `int` | 当前世界帧号 |
| `$player_x` | `fx` | 1P(players[0])的 x 坐标——恒读 1P,不查存活;它是坐标读、不是瞄准原语,瞄准用 aim_player |
| `$player_y` | `fx` | 1P(players[0])的 y 坐标——恒读 1P,不查存活;它是坐标读、不是瞄准原语,瞄准用 aim_player |
| `$self_x` | `fx` | 任务 owner 的 x——敌→敌池坐标，弹→弹池坐标，关卡(STAGE)→0 |
| `$self_y` | `fx` | 任务 owner 的 y——敌→敌池坐标，弹→弹池坐标，关卡(STAGE)→0 |
| `$self_hp` | `int` | owner 当前血量——仅敌(ENEMY)有意义，其余 owner 种类恒 0 |
| `$self_hp_max` | `int` | owner 上限血量——仅敌(ENEMY)有意义，其余 owner 种类恒 0 |
| `$self_age` | `int` | **任务**(不是 owner 实体)出生以来的帧数，对全部 owner 种类(含关卡)均有意义 |
| `$self_vx` | `fx` | owner 的笛卡尔速度 x 分量(px/帧)——敌→敌池，弹→弹池，其余 owner 恒 0 |
| `$self_vy` | `fx` | owner 的笛卡尔速度 y 分量(px/帧)——敌→敌池，弹→弹池，其余 owner 恒 0 |
| `$self_speed` | `fx` | owner 的速率(作者视图，与 $self_vx/$self_vy 恒同步) |
| `$self_angle` | `angle` | owner 的朝向(作者视图，BAM)。**类型是 angle 不是 fx**——能直接喂 move_angle/fire，但与 fx 之间没有隐式转换；近乎静止时不更新(回填有速度下限)，零速下读到的是最后一次有效朝向 |
<!-- gen:engvars:end -->

速度那四个（`$self_vx`/`$self_vy`/`$self_speed`/`$self_angle`）是敌人运动动词族刀
（2026-07-31）加的，全部**读活值**，不是发起动词那刻的快照：速度插值在飞的过程中逐帧读会
读到中途值。它们让"相对运动"不需要专门的 Rel 版动词，见 [3 · 敌人](3-enemy.md)「敌人运动」节的 composed 写法。

`$self_angle` 在**近乎静止**时不更新（回填有速度下限），故零速下读到的是"最后一次有效朝向"
而不是垃圾角——这条是刻意的，否则停一帧就会把朝向抹掉。

## 引擎常量（编译器预置注入，C14）

编译器在处理脚本自己的 `const`/`xformdef`/`sub` **之前**，先把一批 Rust 侧命名常量当作
"第 1 行前已声明的 `const`" 预填进类型检查的常量表。脚本表达式、`const` 初始值、xformdef
槽参数（编译期常量位置）处都能直接引用，不用再手写字面量镜像：

| 名字 | 值 | 含义 |
|---|---:|---|
| `GVAR_RANK` | `0` | `globals` 系统段内 RANK（难度）槽**号** |
| `RANK_EASY` / `RANK_NORMAL` / `RANK_HARD` / `RANK_LUNATIC` | `0`/`1`/`2`/`3` | `global(GVAR_RANK)` 的四个合法**取值**（编号冻结，顺序即难度序，可 `>=` 比较）|
| `RANK_EXTRA` | `4` | 预留档位号 |
| `GLOBALS_SYS_SEGMENT` | `16` | `globals` 系统段/自由段分界槽号 |
| `REQ_*` | 见 `consts.rs` | 通道 B 引擎保留请求 id（`REQ_STAGE_CLEAR`/`REQ_BGM`/…） |
| `ITEM_POWER` / `ITEM_POINT` / `ITEM_LIFE_PIECE` / `ITEM_BOMB_PIECE` / `ITEM_STAR` | `0`/`1`/`2`/`3`/`4` | 道具类型号（编号**冻结**，非表驱动），`drop_add(type, n)` 的第一参 |
| `SHOOTERS_PER_TASK` | `4` | 每任务的发射器槽数——`sh_*` 族槽号 `id` 的**上界**（合法 `0 ..= SHOOTERS_PER_TASK - 1`）|
| `TIMESTOP_FRAMES` | `180` | 玩家技能"时间停止"的固定时长（帧，3 秒 @60Hz）——自机能力刀 spec §9.1，数值单一来源 = `crate::player::TIMESTOP_FRAMES` |
| `JUMP_FRAMES` | `30` | 跳躍跨过的帧数，也是観測影子世界的预览步数——时间机制内核刀 spec §2.2，数值单一来源 = `crate::player::JUMP_FRAMES` |
| `REWIND_DEPTH` | `30` | 遡行落点深度（落点 = 被弹帧 − 本值，钳到快照环最老一帧）——单一来源 = `crate::timeline::REWIND_DEPTH` |
| `BULLET_COLOR_STRIDE` | 内建 `16` | **表派生**：当前绑定表的每种弹型色数 |

脚本**不得**重新声明同名 `const`，无论写的值是否一致——会在类型检查阶段报错
`'NAME' 与引擎常量重名，不能重新声明`（`typeck/consts.rs`）。

**弹型名与颜色名不是引擎常量**（旧的 `APPEARANCE_*` 已随颜色轴刀退场）。它们归**内容包**，
由你自己的 `.ecl` 用 `const` 声明（示例见 `godot/ecl/demo/bullets.ecl`）；同一编译单元
（= 同一目录）内 `const` 跨文件可见，整局脚本只需要在一个文件里声明一次。这样 mod 作者与
内建内容地位对等。写"轮转全部颜色"用 `BULLET_COLOR_STRIDE`，别硬编码 16。

⚠️ **稀疏弹型**（只做了部分色）的其余列是图集空格，盲目轮转全色会被编译期或运行期拒收，
详见 [4 · 弹](4-bullets.md)「部分设三兄弟」。

<details><summary>常量表怎么维护：engine_consts! 宏、类型携带口径、表绑定 content_hash</summary>

值即引擎侧同名 Rust 常量（脚本侧统一按 `int` 携带：`Fx`/`Angle` 类型的常量会带原始
raw 值，不是十进制含义值——目前表里的名字都恰好是 `int` 类型，无此坑；新增 `fx`/`angle`
类型的引擎常量时留意）。

权威定义是 `stg-core` 的 `crates/stg-core/src/consts.rs`，`engine_consts!` 宏对每行同时
生成 Rust 侧 `pub const`（供世界层/harness 代码用，类型保真）与注入表
`ENGINE_CONSTS: &[EngineConst]`（脚本侧统一 `i32`，供编译器 `lang::compile` 默认注入）——
两头共享同一处字面量，杜绝手写复写漂移。**加一个新的引擎常量 = 在该宏调用里加一行**；
C11（`WorldTables` 文件加载）落地后，appearance/道具等表驱动的常量可能改由数据文件
（连同其 `content_hash`）生成而非手写宏调用，命名注入的使用方式不受影响。

`.ecl` 编译现绑定一张表（`compile`/`compile_for_table`）：编译产物 `EclImage` 记录该表的
`content_hash`，运行时若加载的表与之不符，`start_main` 拒绝启动（`TableImageMismatch`）——
这是 C14 记档的"注入常量与运行期表必须同源"这条 coherence 不变量的机制化，见
`docs/follow-ups.md` C11/C14。

</details>

## 内建函数（生成段）

签名以下面这段为准。它由 `cargo run -p stg-harness -- gen-ecl-meta` 从 `builtins.rs` 的
`Builtin.doc` / `param_names` 渲染，**勿手改** `<!-- gen -->` 之间的内容——改动 builtin 元数据
请去改 `crates/stg-ecl-compiler/src/lang/builtins.rs` 再重跑生成器，否则会被
`committed_doc_segment_matches_generated` 防漂移测试打回。

<!-- gen:builtins:begin -->
- `fire(shape: int, color: int, x: fx, y: fx, speed: fx, angle: angle, xf: xform|none, task: sub|none) -> int` — 发一颗弹;shape/color 查外观表(越界/空格 编译期或 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1
- `batch(shape: int, color: int, x: fx, y: fx, n_angle: int, angle0: angle, angle_step: angle, n_speed: int, speed0: fx, speed_step: fx) -> int` — N-way 批量发环;shape/color 同 fire;返实际创建数
- `spawn_enemy(x: fx, y: fx, hp: int, drop_table: int, score: int, sprite: int, task: sub|none) -> int` — 造敌;判定 12/16 默认;task 为敌主任务 async sub 名或 none(owner=新敌;敌死任务亡,任务跑完敌也亡——静默退场,不掉道具不发死亡事件);返敌号(不透明值,别猜数值/别做算术;两个敌号相等 ⇒ 同一只敌),失败 -1
- `enemy_hp(handle: int) -> int` — 查敌当前 hp;死/悬垂/越界句柄返 -1(P4-b;敌号带 generation,槽被另一只敌复用后旧号照样返 -1)——stage 编排等 boss 死用
- `drop_item(x: fx, y: fx, item_type: int) -> int` — 掉一颗道具(带随机喷发速度,消耗模拟 RNG);返句柄,失败 -1
- `move_to(dur: int, x: fx, y: fx, easing: int)` — 敌自身(self owner 非 ENEMY → Fault)按 easing 缓动、dur 帧内平移到 (x,y);四参数皆真实压栈(不同于下方弹 setter 族的占位 handle 首参)
- `move_vel(dur: int, angle: angle, speed: fx, easing: int)` — 敌自身(self owner 非 ENEMY → Fault)按 easing 在 dur 帧内把速度缓动到「angle 方向、speed 速率」;dur=0 = 立即设。**极坐标空间插值**(匀速扫弧,速率按曲线走)——要笛卡尔直线插值用 move_vel_xy
- `move_vel_xy(dur: int, vx: fx, vy: fx, easing: int)` — 同 move_vel 但收笛卡尔分量,且 dur>0 时**在笛卡尔空间插值**(两分量各自线性插,中途速率会掉——线性缓动即恒定加速度);要匀速转向用 move_vel。保住一轴的写法:move_vel_xy(30, $self_vx, 4.0fx, 2)
- `move_angle(dur: int, angle: angle, easing: int)` — 只转向、速率一字不动;dur>0 走**最短弧**(350deg→10deg 走 +20deg 不走 -340deg)。相对转向:move_angle(60, $self_angle + 15deg, 3)
- `move_speed(dur: int, speed: fx, easing: int)` — 只调速、方向一字不动。相对加速:move_speed(30, $self_speed * 2.0fx, 2)
- `set_anm_state(state: int)` — 敌自身(self owner 非 ENEMY → Fault)写表现状态号 anm_state 并无条件盖 anm_state_frame=当前帧(同状态重设=重播,即 ZUN anmInterrupt 的电平版);世界不解释状态号,表现层按 (sprite,anm_state,state_age) 选帧
- `boss_set(slot: int, hp_ratio: fx, spell_id: int, timer_frames: int, phase_left: int, active: int)` — 整槽写 boss_ui 公告板(脚本写/UI 读);enemy 字段取自 self owner(非 ENEMY → NULL,不 Fault);符卡 active 期 enemy/spell_id/timer_frames/hp_ratio 由引擎逐帧自动覆写,phase_left 不受影响仍归脚本
- `pulse_signal(channel: int)` — 脉冲一条信号通道(边沿语义,仅当帧有效);放行处于弹变换 WAIT_SIGNAL 停驻态的弹(非 ECL 任务)
- `emit_req(id: int, a0: int|fx|angle, a1: int|fx|angle, a2: int|fx|angle, a3: int|fx|angle, a4: int|fx|angle, a5: int|fx|angle)` — 通道 B 渲染请求;void 只能裸语句;args 裸载荷(fx 过 raw/angle 过 BAM/int 原样)
- `fx_at(x: fx, y: fx, kind: int, param: int)` — 在 (x,y) 起一次性演出:发 REQ_FX_AT [x raw,y raw,kind,param,0,0](对应 ZUN anmPlayPos);kind/param 引擎不解释,归内容包与壳侧约定;owner 无限制;即发即忘
- `fx_on(kind: int, param: int)` — 在敌自身上起依附演出(self owner 非 ENEMY → Fault):发 REQ_FX_ATTACHED [index,gen,kind,param,0,0](对应 ZUN anmPlay);壳侧按句柄每帧跟随、句柄失效即自毁;即发即忘
- `rand(n: int) -> int` — 模拟 RNG 均匀 [0,n);确定性,随快照回卷
- `global(slot: int) -> int` — 读 globals 槽(GVAR_RANK=0 为难度)
- `set_global(slot: int, value: int)` — 写 globals 槽;系统段(slot<16)脚本写为 no-op+计数,不 Fault(GVAR_RANK=0 建议脚本只读)
- `aim_player() -> angle` — 自身(敌/弹属主)指向**最近可瞄自机**的 BAM 角;一个可瞄的都没有则回退 1P 的最后坐标
- `atan2(y: fx, x: fx) -> angle` — 任意向量的方向角(整数 CORDIC,16 轮);参数序 (y, x) 同 libm;(0,0) 返 0 不报错;比 aim_player 通用——能瞄任意点
- `dist(dx: fx, dy: fx) -> fx` — 向量 (dx,dy) 的模长(开根,不是平方);**不是两点距离**——两点距离自己减: dist(bx-ax, by-ay)
- `nearest_enemy(x: fx, y: fx) -> int` — 离 (x,y) 最近的活敌(非 dying;并列取低索引);无敌返 -1;返的敌号与 spawn_enemy 同编码,可直接喂 enemy_alive/enemy_hp/enemy_x/enemy_y(带 generation,槽复用后旧号可辨);它已排除 dying,故拿到的号过几帧可能已变 dying——该重查而不是继续用
- `enemy_x(handle: int) -> fx` — 按敌号读 x;死/悬垂/越界/槽已被别的敌复用 → 返 0(**不是哨兵**——0 是合法坐标,先用 enemy_alive(e) == 1 探活再读)
- `enemy_y(handle: int) -> fx` — 按敌号读 y;死/悬垂/越界/槽已被别的敌复用 → 返 0(同 enemy_x,先探活再读);配 enemy_x + atan2 即可朝任意敌开火
- `enemy_alive(handle: int) -> int` — 敌号是否仍指向**它当初那只敌**,返 1/0(探活首选,比 enemy_hp(e) != -1 稳——血量恰为 -1 的活敌不会被误判;敌号带 generation,槽被另一只敌复用后旧号返 0);**含正在死的敌**(判的是槽有效不是还能打)
- `sin(angle: angle) -> fx` — 查表三角,返 fx(VM op 直发,非 syscall)
- `cos(angle: angle) -> fx` — 查表三角,返 fx(VM op 直发,非 syscall)
- `set_speed(handle: int, speed: fx)` — 弹 setter:改速率;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `set_angle(handle: int, angle: angle)` — 弹 setter:改方向;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `turn(handle: int, delta: angle)` — 弹 setter:转向增量;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `set_vel(handle: int, vx: fx, vy: fx)` — 弹 setter:直设速度向量;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `set_ang_vel(handle: int, w: int)` — 弹 setter:角速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(POLAR_FX:每帧 angle+=w)
- `set_accel(handle: int, a: fx)` — 弹 setter:切向加速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(POLAR_FX:每帧 speed+=a)
- `set_gravity(handle: int, gx: fx, gy: fx)` — 弹 setter:直角加速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(CART_FX:每帧 v+=(gx,gy);与 POLAR_FX 互斥)
- `stop_fx(handle: int)` — 弹 setter:停连续效果;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(清 POLAR_FX/CART_FX 连续效果)
- `aim_at_player(handle: int, offset: angle)` — 弹 setter:指向自机方向再加 offset 偏移角;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `spell_begin(slot: int, spell_id: int, pattern: sub|none, time_limit: int, bonus0: int, flags: int, hp_threshold: int)` — 开卡:绑 boss/血线/计时/计分,spawn pattern 为卡绑定模式任务(随卡生死)
- `spell_end()` — 手动收卡(取卡按血线自动判,通常不需要)
- `spell_timer() -> int` — 当前卡剩余帧数
- `add_score(delta: int)` — 给自机记分:delta 允许负(扣分),饱和钳 [0,u64::MAX] 不回绕;关底 bonus/结算记账用
- `bgm(id: int)` — 声明当前 BGM:写世界锚点字段 bgm_id 并发 REQ_BGM;mark 跳入自动补偿最近声明(常量参)
- `bg(id: int)` — 声明当前背景:写锚点 bg_id 并发 REQ_BG;换背景隐含新的 phase 纪元(补偿细则见 ecl-lang)
- `bg_phase(phase: int)` — 声明背景演出段号:写 bg_phase 并自动盖 bg_phase_frame=当前帧,发 REQ_BG_PHASE;表现层按段内局部时间 seek
- `time_stop_player(frames: int)` — 停住自机的时间 frames 帧(自机不能动/不能发新弹,自机弹也冻住;敌方照跑);0 = 立即解除;重入覆盖;越界 no-op+计数
- `clear_bullets()` — 全场清弹:铺一个覆盖全场、存活 1 帧的消弹区(复用 FieldPool),每颗被消的弹原位转一颗星星(M0-15);不给护盾帧
- `add_lives(delta: int)` — 增减残机:delta 允许负,双边钳 [0,255] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_
- `add_bombs(delta: int)` — 增减 bomb 数:delta 允许负,双边钳 [0,255] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_
- `add_power(delta: int)` — 增减火力:delta 允许负,双边钳 [0,POWER_MAX=400](即显示 4.00,不是 u16::MAX);开局初值走 Loadout
- `add_time_stops(delta: int)` — 时停次数增量;同 add_lives 语义(允许负、饱和加、钳 [0,255])
- `drop_clear()` — 清空自身待掉落计数;self 必须是敌
- `drop_add(type: int, n: int)` — 自身待掉落计数增量加 n 颗 type(只增不减,要清空用 drop_clear);计数上限 255 饱和
- `drop_items()` — 立刻撒出自身待掉落计数;**吐完不清空**(故 drop_items();die(); 掉双份);不加分不发死亡事件
- `die()` — 就地阵亡:掉落+加分+死亡事件+死亡特效,并**立即终止本任务**(后续语句不执行)
- `sh_reset(id: int)` — 重置发射器槽 id 为默认(1×1 单发、无 xform/挂弹任务/请求)
- `sh_sprite(id: int, shape: int, color: int)` — 设发射器的弹型与颜色;查外观表(越界/空格 编译期或 Fault)
- `sh_offset(id: int, x: fx, y: fx)` — 设出弹点**相对 owner** 的偏移;与 sh_offset_abs 写同一对字段,后写的赢(本条清绝对位标志)
- `sh_offset_abs(id: int, x: fx, y: fx)` — 设出弹点的**绝对**坐标(不跟随 owner);与 sh_offset 写同一对字段,后写的赢
- `sh_offset_rad(id: int, angle: angle, r: fx)` — 设出弹点的极坐标偏移;与 sh_offset/sh_offset_abs **永远叠加**,不是覆盖
- `sh_dist(id: int, d: fx)` — 出生后沿**各自角度**把弹推出去的距离(逐颗方向不同,不是整体平移)
- `sh_angle(id: int, angle0: angle, step: angle)` — 设基准角与逐弹角增量;开了 sh_aim 时 angle0 是相对自机方向的偏移,开了 sh_ring 时 step 转义成逐层偏移
- `sh_speed(id: int, speed0: fx, step: fx)` — 设基准速度与逐层速度增量(层数 = sh_count 的 n_speed)
- `sh_count(id: int, n_angle: int, n_speed: int)` — 设发弹阵列规模:角度向 n_angle 颗 × 速度向 n_speed 层;双边钳 [0,255] 不回绕
- `sh_aim(id: int, on: int)` — 开/关自机狙(on!=0 为开):开则 sh_angle 的 angle0 是相对自机方向的偏移,而非绝对方向
- `sh_ring(id: int, on: int)` — 开/关整周环(on!=0 为开):开则 n_angle 颗自动均分整周;关则是以基准方向为中心对称展开的 fan
- `sh_xform(id: int, xf: xform|none)` — 给发射器挂 xformdef(名或 none);开火时每颗弹都带上
- `sh_task(id: int, sub: sub|none)` — 给发射器挂弹任务 async sub(名或 none);开火时每颗弹都派一个,owner=该弹
- `sh_req(id: int, req_id: int)` — 设开火时顺带发的通道 B 请求 id(音效等);0 = 不发
- `sh_fire(id: int)` — 用发射器槽 id 的参数开火;无返回值;池满走 P4-a 计数
<!-- gen:builtins:end -->

---

**下一篇** → [8 · 报错与限制](8-errors.md)：什么会 Fault、什么静默降级、怎么最快定位。
