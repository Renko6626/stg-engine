# 技术债与待办清单

> **这是什么**：历次代码复审判定「可延后」的发现，逐条核实后的存活清单。
> **为什么在这**：这些发现原本只活在 `.superpowers/sdd/progress.md`（**git-ignored 的临时账本**），
> 后续者看不到、`git clean -fdx` 一下就没。持久的部分必须入库。
>
> **维护规矩**：解决一条就删一条（别留"已完成"的墓碑，git log 才是历史）。新增的复审 follow-up
> 往这里写，别只写账本。**写之前先核实**——本清单每条都经过代码核对，不是复述当年的复审原文。
>
> 最后核实：2026-07-26（Godot 场景刀收口：A5 整条销——乙案 `spawn_enemy` task 参/`enemy_hp`
> 已落地，练习模式定式移一句进 `docs/ecl-lang.md`；B18 收窄为独苗（`visible_instances`）；
> B22 销（两处 smoke 脚本已对称修复）；B23 补全另两对实测 UV 配对；B16①③④ 复核触发点仍未到，
> 措辞刷新；新记 A6-A9/B24-B25/D9/F4——场景刀本刀发现的债：`.ecl` 导出 PCK 过滤坑/宿主读档口
> 缺失/GAME_OVER 三态裁定/演出打磨小件四包（以上四条 A 组）、冒烟 `enemy_seen` 判别盲区/
> `spawn_enemy`&`fire` task 号三拒绝支路零测试（以上两条 B 组）、敌"主协程返回即自燃"承诺
> 未落地为代码（D9）、逐帧全缓冲上传性能账未记（F4））
>
> 终审全分支修复波追记（2026-07-26）：A8 措辞刷新（残机耗尽最小处置已落地，深度流程降级
> 待办）；A9 补第⑤件（effects.gd 三处）；F4 数值订正（468.75KB→468KB整=479232B）；新记
> **B26**（B18/B23/DoD 可玩目验三件套合并单，均卡在"首个有 GPU/X 环境"）。
>
> 弹幕颜色轴刀收口追记（2026-07-26）：B23/B26①追注占位图集已改上下不对称（判决程序不变，
> 只是不必再临时加测试格）；C11 追注 `color_stride` 已进表、乙案符号段仍开放；C14 追注
> `APPEARANCE_*` 系 ② 段符号已随本刀退场（`APPEARANCE_MEDIUM`/`APPEARANCE_STAR=3` 两处
> 历史举例订正为当前状态），注入面现为"①⧺②(空)⧺表派生 `BULLET_COLOR_STRIDE`"；新记
> **B27**（`OP_SET_SHAPE` 缺近 `i32::MAX` 溢出判别式测试，与 `OP_SET_COLOR` 不对称）、
> **C22**（`tables.rs::validate` 的 ② join 校验循环随 ② 段清空而空转，机制保留但非活防护）、
> **D10**（部分设运行期不查 `valid`，编译期空格闸只覆盖 `.ecl` 源码路径，直接构造
> `XformSlot` 的未来消费者不受保护）。

---

## A. 有触发条件的（动到对应模块前先做）

> **判据**：某条债一旦满足"下一刀正好要改这块代码，而这块代码没有网"，就升到 A 组、开工前先还。
> bomb 那刀要改 `world/player.rs` 的生死状态机 —— 这正是当初把 GAMEOVER 缺口升到 A 组的理由。

---

### A3. `new_game` 硬绑内建 `TABLES_V0`——owned 表路径到来时补 tables 参数化姊妹入口（前置小刀终审分诊，2026-07-24）

正典 boot `World::new_game`（step.rs）内部走 `World::new` → 内建 `TABLES_V0`，签名不收
`tables`。v1（内建表唯一）自洽；但当 C11 资产管线的 owned 表（`from_bytes` 载盘、
`content_hash` LIVE）成为消费路径时，这些消费者只能退回 `new_with_tables + set_var +
start_main` 三步——"唯一正典入口"的防分歧保证对该路径有洞。（2026-07-25 追注：正典
完全体现为 `new_game_at`，它同样硬绑 `TABLES_V0`——character 越界检查/重 spawn/标记表
跳转全走内建表——本条的 tables 参数化届时应落在 `new_game_at` 身上、`new_game` 链式
委托。）**触发点 = owned 表消费者出现**（C11 资产管线刀 / 任何 mod 表加载）。

### A4. 背景 STD 式 mini-VM——表现层解释器 + 文本格式（整局流程刀 spec §7 记档，2026-07-25）

机器模型已拍板采纳 ZUN STD 式（`goto label @ time` 同时设 ip 和时钟，纯控制流静态、任意帧
状态可解析求出）；本刀只交付 §4 表现锚点四字段契约（`bgm_id`/`bg_id`/`bg_phase`/
`bg_phase_frame`），解释器本体（连同其文本格式）**不做**——它是**表现层资产**：住
`stg-godot`/Godot 侧，可用浮点、不进校验和、不占任务槽，断层线判据同 F1。**触发点 = Godot
场景刀真做背景演出时**：落地时钉死"嫁接 phase 分段保寻位"这条约定——背景脚本按
`bg_phase` 切段，段内 wait/loop/jump 随便，跨段转移只认 `bg_phase` 边沿；寻位配方
`local_t = 世界帧 - bg_phase_frame`，从该段入口重新解析执行到 `local_t`（段是无记忆的，
读档/中段启动后历史丢失也能重建；变长 boss 段 = 段尾无限 loop、phase 切换破环）。详见
`docs/superpowers/specs/2026-07-25-game-flow-midstart-design.md` §7。

### A6. `.ecl` 是非 Godot 原生资源扩展名——导出 PCK 须显式 `include_filter`（场景刀 T7 记档，2026-07-26）

`godot/ecl/demo/*.ecl`、`crates/stg-godot/smoke/*.ecl` 对 Godot 编辑器/导出器而言是未注册
资源类型的普通文本文件；`_boot` 早前的内置源回退分支已在 T6 删除（demo 目录已实存，静默
降级判为雷），现在纯靠 `DirAccess.open("res://ecl/demo")` 读磁盘文件，读不到就是硬失败（`push_error` +
`return false`），没有任何兜底。本刀只跑 headless/编辑器内路径，从未真正导出过 PCK——
Godot 默认导出规则按已注册资源类型 + 场景引用链收集文件，`.ecl` 两头都不占，大概率被
默认导出规则漏掉。**触发点 = 第一次做 `godot --export` 打包**：导出预设（`export_presets.cfg`）
必须给 `res://ecl/` 显式加 `include_filter`（如 `*.ecl`），否则打出的包在真机上找不到关卡
脚本，且是那种"编辑器里/命令行 `--path` 跑正常，导出包一运行就崩"的隐蔽失败模式。

### A7. 宿主读档口未建——`main.gd` 无 `load_state` 路径（场景刀 T7 记档，2026-07-26）

`bridge.rs` 的 `load_state`/`save_state` 桥面口子（C17）已就位，但 `main.gd` 从未调用
`load_state`——本刀只有 `new_game_at`（含中段启动）一条开局路径，没有"读一份存档继续"的
UI/按键。`_sync_anchors`（双表示规矩：电平追平 hud/bg）目前的唯一调用点是 `_boot` 之后，
职责被 `new_game_at` 路径顺手覆盖了（新开局本就要对表一次）；它真正**不可替代**的场合是
`load_state` 成功之后——读档不像开局，没有 `bgm`/`bg`/`bg_phase` 声明式脚本语句重新跑一遍，
必须靠 `_sync_anchors` 读 `anchors()` 把 HUD/背景电平拉回存档那一帧的状态。**触发点 = 宿主
侧真正要做"继续游戏"/存档菜单时**，届时 `_sync_anchors` 直接复用，缺的只是调用点与 UI。

### A8. `GAME_OVER` 态——spec 四态，实施三态（场景刀 T7 记档，2026-07-26；终审修复波部分兑现）

`docs/superpowers/specs/2026-07-25-godot-scene-design.md` §6 写的状态机是
`PLAYING / PAUSED / STAGE_CLEAR / GAME_OVER` 四态；`main.gd` 的 `enum S` 仍只有三个，没有
新增独立的 `GAME_OVER` 态。但**残机耗尽已有宿主侧最小处置**（终审修复波 I-1）：`_after_step`
侦测 `hud_player().life_state == 4`（`LIFE_GAMEOVER`，`stg-core/src/player.rs`）后复用既有
`STAGE_CLEAR` 三态，切状态 + `hud.show_banner("GAME OVER  (Z restart)", 3600.0)`，拦住了
"残机打光却无任何反馈、宿主继续当胜利处理"这个假胜利缺口——`world/player.rs` 的生死状态机
本身按既有设计正常演化（`LIFE_DEATHWINDOW`→...），只是复用而非新增状态，也没有区分
"胜利结算"与"落败"两条横幅之外的任何后续（continue 续命、计分对齐胜利/落败两条路径的
account 差异）。**触发点 = 已部分兑现**（假胜利已拦，本条降级为深度流程债）：真实内容期若
要做 continue 续命/落败与胜利分道的计分对齐，再回来把 `GAME_OVER` 升格为独立第四态。

### A9. 演出打磨小件五包——spec 写了、实施未接（场景刀 T7 记档，2026-07-26；终审补第⑤件）

`docs/superpowers/specs/2026-07-25-godot-scene-design.md` §7 的请求分发表比 `main.gd`
`_wire_requests` 实际落地的处理器多写了几笔，均属可玩性不受影响的表现层打磨：
① `REQ_SPELL_RESULT`——spec 写"取得/失败横幅 + bonus 飘字"，`main.gd` 只有横幅，没有
`effects.gd` 式的 bonus 数字飘字；② `REQ_BGM`——spec 写"HUD 曲名标签 + 日志（无音频资产）"，
`main.gd` 只更新了标签，没有日志行；③ `hud.gd::refresh` 的 `p.is_empty()` 早退分支只
`return`，不清空 `boss_bar`/`spell_l`——若某帧 `hud_player()` 返回空（如未开局态被意外调用），
boss 条/符卡行会残留上一次刷新的陈旧值而非归零；④ `hud.gd` 的 `banner`（横幅，位于
`Vector2(120, 200)`，无 `word_wrap`/宽度限制）与右栏 HUD 面板（`x≥424`）之间没有互斥/换行
处理，长文案（比如更长的符卡名/中文结算文案）视觉上可能压到右栏；⑤ `effects.gd`——
`_Ring._process` 里 `queue_free()` 之后同一帧仍跑到 `queue_redraw()`（该次重绘是 no-op 但
语义上有点怪）、`dispatcher.gd::drain(arr)` 的 `arr` 参数无类型标注、且 `main.gd` 的 Z 重开
路径（`_boot(0)`）不清空 `effects` 节点下遗留子节点（重开瞬间前一局还没播完的爆炸环/飘字
会带着旧世界坐标残留，直到自身计时器跑完才消失）。五条都不阻塞可玩性，**触发点 = 内容与
美术期**顺手一并做。

## B. 测试覆盖缺口

### B1. P4-a 池满降级：四个写 API 仍零覆盖（道具池份额已还）

`create_bullet` / `create_player_shot` / `create_enemy` / `create_field` 的池满分支
（→ `Handle::NULL` + `diag.pool_full[池id]` + `last_status`）**全无测试**，且金向量也从不触及
（实测稳态：弹 ~375/8192、敌 3/256、field 1/16）。第五个写 API `drop_item`（道具池）的池满分支
已于 M0-12 补测（`step.rs::drop_item_bad_type_and_pool_full`），不再计入本条缺口。

P4-a 是宪法级不变量（资源耗尽 → 确定性降级不 panic，计数入校验和），却是四个池零覆盖。
**建议**：一刀补齐四个池满测试 + 一个把某池打满的金向量饱和场景（后者顺带验证"两机同序同丢"）。

### B2. `push_event` 的溢出分支无测试

`world.rs` 的 `push_hit` 溢出（`diag.hits_overflow`）有测试；结构同构的 `push_event`
（`diag.events_overflow`）没有。补一个镜像测试即可（置 `events_len = EVENTS_CAP`，push，
断言未增 + 计数 +1）。

### B3. 多自机 graze 位隔离 —— 被 co-op 阻塞

`grazed_by` 是每自机一位的掩码。「P0 擦到的弹不该置 P1 的位」目前无测试，因为
`players[1]` 在所有场景里恒为 `LIFE_ABSENT`。**需 co-op 出场机制才可测**，届时一并做。

### B4. overkill 测试只断言事件数，未断言 hp

`settle_overkill_two_shots_one_death_event` 断言 `events_len == 1`，但没断言敌人 hp
**没被二次扣减**。加一行 assert 即可，能多抓一类回归（dying 门禁被绕过但事件恰好只发一次）。

### B5. 碰撞的边界相等（`d2 == sum²`）无测试 —— **低价值，可能永远不做**

M0-8 最终复审的分诊：比较两侧都是精确的 i64 Q32.32 同域整数，**边界在数学上已经钉死**，
补测试只是钉住 `<=`（含边界即撞）这个**约定**，而非防任何精度风险。列在此仅为存档；
真要做也就是几行，但别把它当"缺口"焦虑。

### B6. integrate delay 门测试未断言 speed 冻结（accel=0 无判别力）（D3 终审分诊）

delay 门测试只覆盖了 POLAR 弹 `angle`/位置冻结，`accel=0` 时 `speed` 冻不冻结两条路都过——
无判别力。下次动 `integrate` 时补一条 `accel != 0` 的腿，让 speed 冻结成为可判别断言。

### B7. 互斥 debug_assert 无 should_panic 覆盖（D3 终审分诊）

模式位互斥的 debug 断言（若存在类似兜底）没有 `#[should_panic]` 测试触发；需 `pub(crate)`
直写 `flags` 构造出违规状态才能测。`phase_guard` 已有先例（同样是 pub(crate) 直写触发），
可照抄该模式补一条。

### B8. `nearest_aimable_player` 并列取低索引（严格 `<`）无两人等距判别测试（D3 终审分诊）

实现用严格小于（`d2 < bd`）保证并列时取低索引（I4 口径），但没有"两个自机等距"的判别测试
去区分 `<` 与 `<=`。被 co-op 阻塞（`players[1]` 目前恒 `LIFE_ABSENT`），与 B3 同期做。

### B9. `contract_viol` 跨类别双计语义未钉（D4 终审分诊）

同一次调用若同时踩中两类契约违规（例如半径越界 + 变换坏参数），当前实现会各计一次、共 +2。
`clamp_radius` 注释里的"只计一次"约定明说的对象是"同一类别里多个半径字段合并算一次"，没有
覆盖"跨类别是否各自计数"这条轴。现状（跨类别各计一次）站得住，但缺一条测试锁死——不锁的话
下次改动可能悄悄把它并成"整次调用最多计 1"而没人发现是语义变化。**建议**：补一条构造出同一
调用内两类违规同时触发的测试，断言 `contract_viol` 恰 +2。

### B10. 金向量⑨/STEP/LOOP 的"fired-region × LOOP 回跳"角落语义缺测试与文档句（11b 终审分诊）

LOOP 跳回后 `[0, xform_next)` 收缩：活跃 STEP 冻结至重武装、武装反弹弹该窗口 walls 读 0
暂时失效——均确定性且属 spec 字面执行，但缺一条判别式测试与一句设计文档明写这条角落语义。

### B11. 越界 `drop_table` id 分支无直测（M0-12 终审分诊）

`create_enemy` 不验 `drop_table`，调用方可传入越界 id 使其在 settle 趟二触达"视同空表 +
`contract_viol` 计数"的降级分支——P4-b 是宪法级不变量，按纪律该分支应有测试，目前无。

### B12. `credit_item` 的 `lives`/`bombs` u8 进位无饱和（M0-12 终审分诊）

`credit_item` 里 `lives`/`bombs` 字段累加没有饱和上限：刷够 1275 枚生命/炸弹蜡（`u8` 满表下
的极端场景）会在 debug 触发溢出 panic、release 静默回绕——确定性不破（跨平台逐位一致仍成立）
但入账值荒谬。修法：累加改 `saturating_add(1)`，蜡数比较从 `==` 改 `>=` 以抗直写越界。

### B13. 道具近距磁吸 v0 的自机选择与优先裁决序偏离 spec —— 与 B3/B8 co-op 族同批（M0-12 终审分诊）

道具近距磁吸 v0 实现取"升序首个圈内自机"，而非 spec 写的"最近的 ALIVE 自机"——`players[1]`
恒 `LIFE_ABSENT`，两种取法在单机场景下不可观测、无法用当前测试区分。另外 PoC 磁吸与近距圈
两者的优先裁决序 spec 未写明，也是同一刀留下的空白。co-op 出场机制到位（解除 B3/B8 阻塞）后
与它们一并定案；此处先把"spec 偏离"显式记档，避免后续误当 bug 修掉。

### B14. `storm --saves 0` 静默空转假绿（L1/L2 终审分诊,2026-07-23）

`run_storm` 的 `save_at` 无 `saves ≥ 1` 守卫也无去重:`--saves 0` 时零存档点、零重演腿,
照样打印"全部逐位一致"退出 0——**正确性工具自身的假绿脚枪**(CI 不走 CLI 路径,只坑手动
调用)。修法:`save_at.is_empty() → Err` + 去重。顺手级。

### B15. `storm_short_gate` 是全套件最重单测(~10.6s debug,终审实测知会)

240 帧 × 3 点的短版闸占 harness 测试总时长 ~全部;ARM CI 更慢。暂不动作;若 CI 墙钟成
问题,减帧或拆 `#[ignore]` 全量版 + 轻量常开版。

### B16. 符卡机构三小件（符卡计器刀终审分诊，2026-07-24）

**已还**（2026-07-25 前置债务刀）：② **boss_ui 结算不清致 ≤1 帧陈旧闪**——`settle_one_spell`
（`world.rs`）结算原子里紧随 `spells[slot]` 清零同步清 `boss_ui[slot]`，卡结算后下一卡相位 7
自动喂前不再残留旧卡 active/spell_id/timer 一帧陈旧值。

① **脚本读自机资源 syscall（原 G1）**：`PlayerState` 有 lives/bombs/score/graze 全套但无读
syscall——被符卡机构溶解后从"符卡前置"降为独立小件（bonus 结算引擎付、收卡事件世界产出,
脚本不再需要轮询资源判 miss）。真做花式收卡条件（如"无擦弹收卡"）时再加一族读号。
③ **ZUN 分段衰减曲线**：v1 线性衰减（begin 时整除定格 dec_per_frame）;真做关卡内容嫌糙再
升级为 ZUN 分段（快衰段+慢衰段+地板），参数已隔离在 begin 计算,不动 syscall 接口。
④ **负 threshold 与 ENEMY_DYING 分支**：syscall 已拒 `threshold<0`（脚本到不了负值）;
`hp_break` 的三路 OR 中 ENEMY_DYING 一路对 threshold≥0 实为死码（=0 冗余、>0 被下钳挡）,仅
threshold<0（world API 白盒可达）承重——保留作"非 damage_enemy 死亡路径"的防御,记档非债。

**场景刀复核（2026-07-26）**：①③④触发点均未到——demo 局收卡（风铃卡）走系统默认结算，未写
任何"无擦弹"一类花式收卡条件；符卡衰减曲线仍是 v1 线性，demo 内容量级未觉得"嫌糙"到需要
升级 ZUN 分段；④本非债、性质不变。三条措辞与代码现状一致，未改动。

---

### B18. `visible_instances`（`MultiMesh` 可见实例数）不可 headless 断言，须真渲染器（桥刀终审分诊，2026-07-24；场景刀收窄为独苗，2026-07-26）

`hud_spell`/`fields_info` 两项判别断言已由场景刀 T6（`crates/stg-godot/smoke/smoke.gd`/
`godot_smoke.ecl` 新增的 `smoke_spell_pattern`/`smoke_boss`）补齐，`register_layer`/编码
上传链回读/`LAYER_SHOTS` 判别已于 2026-07-25 前置债务刀清账——本条收窄为独苗。**两层都
不可断言**：①`MultiMesh` 资源对象自身的 `visible_instance_count` 字段是客户端本地缓存，
桥面走 `RenderingServer.multimesh_set_visible_instances` 直写服务端从不经资源 setter，
该字段永远停在 `playfield.gd::_make_layer` 播种的初值（0），读它必错；②退一步走服务端
真值 `RenderingServer.multimesh_get_visible_instances`，headless dummy renderer 下同样
实测恒 0（`smoke.gd` 注释"已实验判决，不可测"，现有冒烟改走 `multimesh_get_buffer` 回读
实数据判别绕开，不断言可见数）。**触发点 = 首个有 GPU/真渲染器的环境**，与 B23 的 UV
判决同批可做（同样卡在"本机无 GPU/无 X"）——合并单见 **B26**（与 B23、DoD 可玩目验三件
一次做完，别单做一件就散场）。

### B19. 清弹 builtin——语义空间未定（整局流程刀 spec §8 记档，2026-07-25）

practice 单场景不需要清弹；真实整局脚本关底转场（`REQ_STAGE_CLEAR` 挂牌前）大概率需要，
但语义有内容层设计空间未拍板：直接消（静默清空）/ 转点（消弹换分/道具）/ 护盾帧（清弹同时
给自机短暂无敌）三种玩法权重不同，`FieldPool` 已有通用消弹区机制（bomb 那刀是它的首个
真租户，见本文件"E. bomb 那一刀开工前"）可复用，缺的是脚本层 builtin 与语义选型。**触发点
= 真实整局脚本落地、关底转场需求出现时**，届时再定语义、开新 syscall 号（5x 族之后的下一个
空号）。

### B20. `set_power`/`set_lives`/`set_bombs` 账面 setter 三件——脚本侧暂无场景（整局流程刀 spec §3 记档，2026-07-25）

装备/命数/炸弹数三件账面 setter 曾在早期草案里设想给脚本用（如"道中事件奖一条命"），但
`Loadout`（`crates/stg-core/src/player.rs`）已把**装备上行**这唯一确定场景收编——菜单侧
practice 装备走 `new_game_at` 的 `loadout` 参数，不需要脚本再写一次。四件套因此**缩编为
一件 `add_score`**（关底 bonus/结算记账必须在世界内发生，其余三个暂无消费者）。**触发点 =
"道中事件奖命/加炸弹"一类脚本需求真出现时**，届时按 `add_score` 同款 5x 族口径（1 参、无
返回、钳位/饱和语义见 P4-b）补开 syscall。

### B21. mark 自动补偿的 `visited` 防环在"同一 sub 被同步调用两次"场景下与纯线性执行序背离（整局流程刀批审 Minor-1，2026-07-25）

`scan_mark_compensation`/`walk_mark_scan`（`crates/stg-ecl-compiler/src/lang/codegen.rs`）
沿 `main` 的同步调用链顶层线性展开时，用 `visited: BTreeSet<&str>` 保证每个 sub 全程**只
走一次**（防环，I4 同款确定性容器纪律）。病态场景：同一个 sub 在 `main` 顶层被同步调用
**两次**，且两次调用之间还有一条顶层锚点声明——例如 `common(); bgm(5); mark(1); common();
mark(2);`（`sub common() { bgm(9); }`）。`visited` 在第一次遇到 `common` 时就把它标记为
已访问，第二次调用点**不会**被重新展开，于是 `mark(2)` 处注入的补偿值仍是第一次展开时
算出的"最新值"（`bgm=5`），而不是"真按顺序回放到这里"会得到的值（`bgm=9`，来自第二次
`common()` 调用）——扫描顺序与**纯线性执行序**背离。影响评估：注入的值不是垃圾/未定义
状态，是脚本里确实声明过的某个值（只是不一定是最近那次）；同一份脚本永远编译出同一张
标记表，确定性/回放/跨平台一致性不受影响；真实整局脚本几乎不会把同一个 sub 在同一条同步
调用链里连续调用两次（ZUN 式关卡是"顺序调不同的关卡 sub"，不是"重复调同一个"）。**修法
方向**：把 `visited` 的粒度从"全局只访问一次"收紧为"按调用点/调用路径重扫"，让同一 sub
在不同调用点各自贡献一次"最新值"快照——真实脚本出现这种写法、或想让补偿更贴近直觉时再做。

### B23. `layer.gdshader` 图集选格 UV 垂直朝向嫌疑——占位图元对称，暂不可判（渲染链刀 T4 复审 Important，2026-07-26；颜色轴刀 T6 追注占位图元已改非对称，2026-07-26）

审阅者本机（无 GPU/无 X）用 `QuadMesh(32,32).get_mesh_arrays()` 静态读出顶点/UV 配对（四对
全部实测，非外推）：`v=(16,-16) uv=(1,1)` / `v=(16,16) uv=(1,0)` / `v=(-16,-16) uv=(0,1)` /
`v=(-16,16) uv=(0,0)`（Godot 2D 里 y 向下，顶点 y=+16 是屏幕下方）。
若这组配对在真渲染管线里如实生效，`layer.gdshader::fragment()` 的
`uv = (cell + UV) / vec2(grid_cols, grid_rows)` 会把每个格**上下镜像**贴到 quad 上——
UV.y=0（贴图该格顶行）贴到 quad 下沿（y=+16），UV.y=1（该格底行）贴到 quad 上沿（y=-16）。
当前占位图集（`tools/gen_atlas.gd`）画的全是上下对称图元（圆/菱形/竖直椭圆/居中方块），
真镜像了也肉眼看不出——**无法用现有占位资源判别**；自机走独立 `Sprite2D`（`playfield.gd`
`_make_player`），不经这条 shader，同样绕不开这个问题域。换真美术、格内图案一旦上下不对称
（比如带朝向的弹幕图元）就会暴露：自机正常、弹/敌/道具全上下镜像。

**不盲修的理由**：本机无 GPU/无 X（`xdpyinfo` 探测失败），`QuadMesh.get_mesh_arrays()` 是
CPU 侧网格数据，不代表 GPU 光栅化管线的最终采样结果（NDC/视口变换/`CanvasItem` 自身坐标系
翻转等中间环节可能已经把这次"镜像"抵消）——没有真渲染器出图对照，盲改等于拿猜测覆盖猜测，
可能把不存在的问题"修"出真问题。

**判决程序**（留给首个有 GPU/X 的环境）：往 `gen_atlas.gd` 临时加一张上下不对称的测试格
（例如上半格纯红、下半格纯蓝），塞进某一层图集第 0 格，有头跑
`godot --path godot`，肉眼看落地画面该格是"上红下蓝"（未镜像）还是"上蓝下红"（镜像）。

**候选修法**（若判决确认镜像，一行改 `layer.gdshader::fragment()`）：

```glsl
vec2 uv = (cell + vec2(UV.x, 1.0 - UV.y)) / vec2(grid_cols, grid_rows);
```

确认后同步在 `docs/render-contract.md` §3 补一条 UV 朝向约定（记录判决结果 + 该行改法）。
合并单见 **B26**（与 B18、DoD 可玩目验三件一次做完，别单做一件就散场）。

**追注（颜色轴刀 T6，2026-07-26）**：占位图集已从"全上下对称图元"改为上下明暗渐变
（`gen_atlas.gd::_disc_shaded`，颜色轴刀 T5，2026-07-26）——上面"当前占位图集…无法用现有
占位资源判别"那句因此已是**历史状态**：现在首个有头启动（`godot --path godot`）即可直接
肉眼判别该层是否镜像，不必再按下方"判决程序"临时加测试格。判决程序与候选修法本身不变
（仍是同一行 `layer.gdshader` 改法），只是判别用的图元已经不需要专门再造了。

### B24. 冒烟①`enemy_seen` 判别不辨 boss/杂兵——负控实证（demo 局刀 T6 复审残余缝隙，2026-07-26）

`main.gd::_run_smoke` 的 `enemy_seen` 断言（逐帧扫 `LAYER_ENEMIES` 缓冲找非默认实例位置）
证明"敌真的在动"，但不区分动的是 `stage1` 杂兵还是 boss——**负控制实测**：临时把
`main.ecl` 整段替换成 `sub main() { bgm(1); loop { wait(600); } }`（不调 `stage1()`/
`boss_battle()`，即"main 只剩 bgm+loop"）会被真实抓到（`SMOKE FAIL`）；但若只清空
`stage1()` 内部逻辑（`zako_dive` 全删）而保留 `main` 的调用序，`boss_battle()` 几乎立即
执行，boss 出生点 `y=96.0` 本身就让 `enemy_seen` 命中，判别面被 boss "顶包"，这类更典型
的"只删杂兵波次"回归**不会**被当前断言捕获（换个说法：只要 `boss_battle` 仍在 240 帧窗口
内被摸到，`enemy_seen` 就不关心杂兵段死活了）。修法方向：加一条按坐标排除 boss 的判据
（如 `|ox|>40`——demo boss 固定 `x=0` 出生，杂兵三波在场界左右两侧出生/俯冲）或改按
敌实例数量（杂兵波次期望瞬时敌数 >1，boss 单敌）判别。**触发点 = 下次真要收紧这条冒烟
断言，或者杂兵/boss 内容有实质变化时**（权衡：复杂度 vs 真实手误极少"连节奏一起清空"，
当前不算阻塞）。

### B25. `spawn_enemy`/`fire` 的 task 号三条拒绝支路合计零测试（场景刀 T1/demo 局刀实现附带发现，2026-07-26）

`sys_spawn_enemy`（`ecl/syscall.rs`）与 `sys_create_bullet`（backs `fire`）的 task 参校验
都是同一套三段先验后建：①`task_script` 超出 `u16` 范围（`u16::try_from` 失败）；②号不在册
（`sub_id` 查无）；③在册但不是零参 `Async`（`kind() != SubKind::Async` 或有参数）——全部
`FAULT_BAD_OP`。现状每个函数**只有②有直接测试**
（`spawn_enemy_bad_task_script_faults_without_enemy`/
`sys_create_bullet_bad_task_script_faults_before_creating`），①③零覆盖，`spawn_enemy`
与 `fire` 两族完全同构地留了同一个缺口（"与 fire 先例一致"——不是 `spawn_enemy` 独有的
新债，是抄了既有代码路径连带抄了既有测试盲区）。P4-b 是宪法级不变量，按纪律该补齐。
**顺带记一句设计口径**：`EnemyPool.main_task` 是**只写不读**的记账字段（`stg-world-design.md`
称其为 Handle 用途，不是"当前存活任务"的活句柄——敌死后任务被 owner-gate 清杀，但
`main_task` 本身不会被清零，读到非零不代表任务还活着；目前全仓也确实没有任何消费者读它，
只是别在将来加消费者时想当然把它当"活任务槽号"用。

### B26. 首个有 GPU/X 环境的三件套合并单（终审全分支终审新记，2026-07-26）

三条独立记档的债都卡在同一个前提——**本机全程无 GPU/无 X**（`xdpyinfo` 探测失败），
且**全刀无一次有头运行**（画面从未被人眼看过，含本条判决程序涉及的自机 `Sprite2D`/
弹幕图元/HUD 排版）——各自记在各自条目里容易各等各忘，合并列一次防止漏项：

① **B23** `layer.gdshader` 图集选格 UV 垂直朝向嫌疑——判决程序：`gen_atlas.gd` 临时
加一张上下不对称测试格，有头跑 `godot --path godot` 肉眼判"上红下蓝"是否镜像（详见 B23
条内判决程序全文）。**追注（颜色轴刀，2026-07-26）**：占位图集本身已经上下不对称
（`_disc_shaded`，颜色轴刀 T5），首个有头启动即可直接肉眼判别、不必再临时加测试格；
判决程序其余步骤（对照 `layer.gdshader` 候选改法）不变。

② **B18** `visible_instances`（`MultiMesh` 可见实例数）不可 headless 断言——判决程序：
有 GPU/真渲染器后重跑 `godot/smoke`，把当前绕开用的 `multimesh_get_buffer` 判据换回
`RenderingServer.multimesh_get_visible_instances` 直接断言，确认非 0（详见 B18 条内
两层不可断言的具体原因）。

③ **DoD 可玩目验**——`CLAUDE.md` Phase 1 之外，本刀（Godot 场景刀 + 本次终审修复波）
的隐性 DoD 是"可玩 + 可验证"，但**验证目前全靠 headless 冒烟断言，没有一帧被人眼看过**：
demo 局的图集贴图是否如预期摆放、HUD 排版是否重叠、boss 战节奏是否真的可打（900hp 是
仓外探针实测数据，见 `godot/ecl/demo/boss_windchime.ecl` 注释，不是本机有头试玩验证的）、
输入手感是否正常——一概未经目验。判决程序：有 GPU/X 环境后 `godot --path godot`（非
`--headless`）跑一局 demo，键盘操作到风铃卡结算，肉眼确认贴图/HUD/节奏均正常。

**触发点 = 同一个**：首个有 GPU/真渲染器/X 环境的会话，三件一次做完（不要只做其中一件
就散场——判决程序共享同一次有头启动成本）。

### B27. `OP_SET_SHAPE` 缺近 `i32::MAX` 溢出判别式测试——与 `OP_SET_COLOR` 覆盖不对称（颜色轴刀 T7 复审修复轮遗留，2026-07-26）

T7 复审修复轮把 `OP_SET_SHAPE`/`OP_SET_COLOR` 两臂的裸 `+` 都改成了 `wrapping_add`
（`world/transform.rs`，见该刀"必修二"）——**实现本身对称**，两臂都不会在 `args[0]` 接近
`i32::MAX` 时 panic。但补的判别式测试
`partial_sprite_ops_wrap_instead_of_panicking_on_near_max_args` 只构造了 `OP_SET_COLOR`
的近溢出槽，`OP_SET_SHAPE` 没有同款腿——如果将来有人把 `OP_SET_SHAPE` 的 `wrapping_add`
误改回裸 `+`，现有测试套件抓不到，得等到某个真实脚本凑巧撞上大数值才会在别处炸出来。
**修法**：照抄该测试的模式给 `OP_SET_SHAPE` 补一条对称腿（`args[0] = i32::MAX` 的
`OP_SET_SHAPE` 槽，手算 `wrapping_add` 后的截断值，断言不 panic + 值落在预期位 + 序列不
终止）。**触发点 = 下次改动 `world/transform.rs` 的 `fire_op` 或该文件近溢出测试组时**
顺手补，不必单独开工。

## C. 代码整洁（低优先，都是两可）

### C1. `world/settle.rs:84,95` —— 两 arm 的门禁 2 行逐字重复

`ROW_SHOT_ENEMY` 与 `ROW_FIELD_ENEMY` 两臂各有一份
`if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 { continue; }`。
两处调用点、2 行 —— 抽 `fn enemy_targetable(&self, e) -> bool` 属过早。
**第三个伤害源出现时再抽**（复审与实现者两次独立判断一致）。

### C2. `world.rs` 的 `OOB_MARGIN` 提了 `pub(crate)` 但只有一个消费者

`FIELD_HALF_W`/`FIELD_HEIGHT` 确有两个消费者（`player.rs` 的场界钳制 + `cleanup.rs` 的越界判定），
`pub(crate)` 是对的。但 `OOB_MARGIN` 只被 `cleanup.rs` 用。可沉为 `cleanup.rs` 私有，
也可为「D7 场界几何三件套聚在一处」留着 —— **两个选择都站得住，别为它开会**。

### C3. `NUM_PHASES` 在 release 构建触发 `dead_code`

只在 `phase_enter` 的 `#[cfg(debug_assertions)]` 块里用。CI 的 clippy 跑 debug 故不报，
但 `cargo build --release` 会。修法：`#[cfg_attr(not(debug_assertions), allow(dead_code))]`。
**先于 M0-9 存在**，非拆分所致。

### C4. `push_hit` / `push_event` ~6 行结构重复

两个缓冲、不同元素类型、不同 diag 计数器。两处调用点抽象属过早 —— 与 C1 同款判断。

### C5. `world/integrate.rs` 的部分规格住在 `world.rs`

`field_life_one_lives_exactly_one_frame` / `field_life_n_survives_n_frames` 测的是
「integrate 倒数 ↔ cleanup 回收」的跨相位时序，留在了既不拥有相位 5 也不拥有相位 9 的 `world.rs`。
`step.rs` 有 step 级兜底，故低急。真要动就挪到 `cleanup.rs` 或 `step.rs`（后者拥有跨相位顺序）。

### C6. `backfill_polar` 的 `isqrt(..) as i32` 理论回绕（D3 终审分诊）

`|v|` 逼近 `Fx` 上限时 `isqrt(len_sq(vx,vy) as u64) as i32` 理论上可回绕为负 `speed`——
确定性无损（跨平台仍逐位一致）、当帧越界回收兜底，纯理论风险。按 P4-c 对称性（引擎自身
bug → debug 帧内断言）补一条 `debug_assert!(sp.raw() >= 0)` 之类的兜底。

### C7. `create_bullet_with_xform` 严格化两件（11b 终审分诊）

STEP `args[1]` 位 24-31 未强制为零（reserved-must-be-zero 前向兼容纪律，未来扩展位若悄悄
非零会被当前实现无声吞掉）；WAIT_SIGNAL 坏通道 / BOUNCE_ARM 坏 n 仅 fire 侧处置（运行期靠
no-op/计数兜底），与 easing id 的 create 期拒收不对称——两条都安全无洞，只是作者体验不一致
（有的坏参 create 时就打回，有的要等 fire 才看见）。

同族补两对（2026-07-23 系统审阅）：`SYS_CREATE_BULLET*` 的 `appearance` 越界 → Fault，而
`SYS_DROP_ITEM` 的 `item_type` 越界 → 静默 NULL+计数；`batch` 负计数 `max(0)` 钳零不计数，
而 `xform_cnt` 负数 → Fault。同为"坏枚举/负参"，脚本作者遇到的响度不可预测——归并本条一起裁。

### C8. `slot`/`xf_bullet` 测试助手三处复制（11b 终审分诊）

`step.rs`/`world/transform.rs`/`world/integrate.rs` 各自维护一份结构相同的 `fn slot`
（构造 `XformSlot`）+ `world/transform.rs`/`world/integrate.rs` 各一份 `fn xf_bullet`
（挂变换序列造弹），三处复制。可归拢进 `world.rs` 的 `test_support`（`bullet_at` 已在
那），非阻塞，两可。

### C9. 负 speed 语义全链无测试钉（M0-14 终审分诊）

`speed` 为负（作者直填，或 `speed_step` 为负跨零，或 `ADD_SPEED` 减过头）时
`polar_to_vec` 方向翻转 180°——单发/批量/变换三入口行为一致且确定，但没有任何测试
钉住这个语义。不是 bug（东方语义里"负速=倒飞"甚至有用），但它是接口契约的无声角落：
若未来有人在 `polar_to_vec` 或某入口加"负速钳零"，全套测试仍绿、行为已变。补一条
单测（负速直填 + 批量负步跨零各一断言）即可钉死，M1 ECL 暴露 `create_bullets_batch`
给脚本作者前值得做。

### C10. homing 自机弹单刀设计注记（M0-17 grill 后置）

shottype 表 `Shooter.flags` bit0 已预留 homing。开刀时要拍的唯一悬案是**转向率存放**：
甲案全局常量（所有追踪弹同转向率，零池改动）；乙案 `ShotPool` 加 `turn_rate` 字段
（池布局变更，校验和自动跟上，表达力全）。integrate 加分支走 `nearest_enemy`（现成）。

### C11. WorldTables 资产管线（已还，2026-07-21）

**已还**：`WorldTables` owned 化（内部 `&'static` 切片 → `Box`，`TABLES_V0` 为
`LazyLock`）；规范字节格式 `to_bytes`/`from_bytes`（i32/u16 小端，无 float，`TableLoadError`）
+ 真 `content_hash`（vendored FNV-1a64，加载时自校验）；`tables_v0.bin`（840B）由 harness
从 `build_tables_v0().to_bytes()` 烘焙、committed，`verify-tables` 逐位对拍，`stg-core` 经
`include_bytes!` + `from_bytes` 加载，`content_hash` 现为 LIVE 值（`0x3ac258d4031d2ced`）；
`compile_for_table(src, file, &WorldTables)` 把表 `content_hash` 焊进 `EclImage`
（`compile` 委派绑定 `TABLES_V0`，签名不变）；`World.tables_hash`（`new_with_tables` 记录，
入校验和，随快照复制）+ `start_main` coherence 守卫（`image.content_hash != tables_hash` 时
拒绝，`TaskStartError::TableImageMismatch`，任一侧为 0 视为未绑定放行，只在启动时查一次）；
harness 端到端自证：从磁盘加载 `tables_v0.bin` 跑一遍（挂弹+移动），校验和与内建路径一致。

**未挡住的口子**：
- `WorldTables::from_bytes` 按**文件里的计数**直接 `Vec::with_capacity(count)`——对**可信**表安全（body
  哈希先验，损坏 → `HashMismatch`；唯二调用者是 committed `tables_v0.bin` 与 harness 对它的测试）。但
  **蓄意构造**的文件（自洽 hash + `count = u32::MAX`）会在读循环撞 `Truncated` 之前触发数 GB 预分配 →
  OOM abort，一条非确定性 panic 路径（与 P4-a 张力）。**加载不可信 mod `.bin` 之前必补**：分配前用
  剩余缓冲长度给每个 count 封顶（终审 2026-07-21 分诊）。

**仍留给未来（modding，非本刀范围）**：
- **`EclImage` 离线序列化格式**（2026-07-23 审阅拍板记档）：M2 走"启动时编译 `.ecl` 源码"
  （harness 同款，`compile_for_table`）；镜像字节格式**不冻结**，离线分发/加载等 modding
  需求真出现再定——过早冻结 = 白背一份格式兼容债。
- **乙案**——表自带 `[(name,id)]` 符号段，让非 Rust/mod 作者自定义外观词表，v1 仍用
  `consts.rs` 里手写的 ② 符号名字。**追注（颜色轴刀，2026-07-26）**：表已带 `color_stride`
  字段（`WorldTables.color_stride`），乙案的**符号段**本身仍未落地——`consts.rs` 的 ②
  段现已清空，弹型名/色名改由内容包自己的 `.ecl` 用 `const` 声明（见 C14、
  `docs/ecl-lang.md`"引擎常量"节），mod 作者靠这条路径已经能给外观取名，乙案不是
  "唯一出路"了；乙案真落地后，这条 `const` 声明的路径可以退位给表自带的机读符号段。
- **文本 DSL** 给表本身当作者格式——v1 仍是 Rust 里建 + 烘焙字节，无文本源。
- 多角色 / 多道具类型表（v1 固定 `characters` 长度 1、`item_cfg` 长度 `ITEM_TYPE_COUNT`）。

### C12. ECL 后续小件四包（M1 T5 分诊）

① `spawn_task_now`（当帧 worklist drain 版）——design_doc §4.3 既定后期可选，真实用例窄；
② 帧相对寻址指令族（sub 可重入）——locals 共享是 v0 拍板，重入需求出现时纯增量加；
③ vm.rs 顶部 `#![allow(dead_code)]` 毯（T1 遗留）与 `run_tasks` 的 256 槽逐位扫描措辞
   （复审 Minor：正确性无碍，可改按字跳空；`KILL_CHILDREN` 同款 256 槽扫、预算只计 1 条
   ——终审 Minor，病态脚本群最坏 ~16.8M 探测/帧，纯性能项）；④ 敌 appearance 表
   （`spawn_enemy` syscall 现走直参，弹的 appearance 表已建——对称化留内容需要时）；
⑤ spec syscall 清单三项**有意未入号表 v1**（终审 Minor 补记）：
   `last_status`/`nearest_enemy`/`attract_all_items`（号表 v2 候补），`SPAWN` 的
   "owner 来源枚举操作数"简化为恒继承——现实现更简且够用，记录在案防"悄悄丢"。

### C13. ECL 表层语言（M1.9 已落地）——销账与遗留

**已还**（M1.9，2026-07-19）：①表达式即参数 ✔（类型化内建 + 运行时表达式直通）；
②值消费静态检查 ✔（`_ =` 显式丢弃）；③repeat 限制 ✔（真 `for`/`while`/`loop`）；
④单位字面量 ✔（`1.5fx`/`90deg`/`bam`）；⑥引擎状态变量化 ✔（`$` 前缀，随机数变量
仍拒绝）。**Named entry ABI**（2026-07-20）：⑧强制 `sub main()` + singleton 生命周期 ✔；
⑨async sub 自动注册为公共 named entry ✔；⑩普通 sub 为 CallOnly（不注册 entry）✔；
⑪CALL/SPAWN 操作数为规范 `SubId`，运行时执行 SubKind 检查 ✔；
⑫安全绑定层（`start_main`/`spawn_entry`/`spawn_entry_named`）替代裸 `spawn_task` ✔；
⑬编译器可选调试符号侧载（`DebugInfo::Full`），`EclImage` 本身不变 ✔。
**遗留**：⑤时间标签 `+N:` 糖（未进 v1，用户拍板显式 wait；备胎保留）；
⑦`jnz` 收编（纯密度优化，降低模板实证不需要）。

### C14. .ecl 跨语言常量引用缺失（M1.9 T4 复审分诊，三 Minor 共同根因）

**已还**（常量注入半，2026-07-21）：`.ecl` 源码现可引用 Rust 侧命名常量——`stg-core`
`engine_consts!` 注册表（`crates/stg-core/src/consts.rs`）对每行同时生成 Rust `pub const`
与注入表 `ENGINE_CONSTS`，编译器 `lang::compile` 默认把它们当"第 1 行前预声明的 const"
注入类型检查命名空间（脚本重声明同名报错"与引擎常量重名，不能重新声明"，见
`typeck/consts.rs`）。原先三份并行的 const 求值/运算规则（typeck 内联折叠、codegen
`eval_const_arg`、`typeck::matrix` 的 `mulf_const`/`divf_const` 等）收编成 `lang::const_eval`
（求值）+ `lang::type_rules`（运算矩阵/cast 白名单），旧 `typeck/matrix.rs`、
`typeck/intents.rs` 已删。金向量场景 `rainbow.ecl` 去魔数示范：风铃摆环固定外观
`fire(1, ...)` → `fire(APPEARANCE_MEDIUM, ...)`（`var appearance = i % 4` 那类有意轮转
全表的写法保留不换）；校验和不变（纯换书写不换值，两次 golden 对拍 + 编辑前后对拍均逐位
相同）。详见 `docs/ecl-lang.md`"引擎常量"节、
`docs/superpowers/specs/2026-07-21-ecl-const-injection-design.md`。**追注（颜色轴刀，
2026-07-26）**：`APPEARANCE_MEDIUM` 一类 ② 段符号已随颜色轴刀退场——`fire`/`batch`
现收两参（`shape`/`color`），`rainbow.ecl` 已改写成 `fire(BULLET_BALL_M, COLOR_CYAN, ...)`
这类调用，`BULLET_BALL_M`/`COLOR_CYAN` 是脚本自己用 `const` 声明的内容包词汇，不再是
Rust 侧注入的引擎常量；上面这句"去魔数示范"描述的是它当时（2026-07-21）的写法，读到本条
时若查 `consts.rs` 找不到 `APPEARANCE_MEDIUM`，不是回归，是这条追注记录的迁移。

**coherence 不变量**（本刀记档，留 C11 焊死）：注入的引擎常量在字节码里落为**字面量值**
（非名字），故存在一条语义前提——**同一份常量来源必须同时喂"编译期注入"与"运行期查表"**。
当前 v0：`ENGINE_CONSTS` 由 `consts.rs` 单一权威生成，`WorldTables`（v0 静态 `TABLES_V0`）
与之同源（如 `APPEARANCE_STAR=3` 既是注入给编译器的值、也是 `TABLES_V0.appearances` 的
索引），二者天然一致，靠的是"同一个宏调用/同一份源文件"这条结构性保证，不是机制性校验。
**C11 表文件加载落地后风险浮现**：若注入常量取自表 A、却拿表 B 跑，appearance id 可能
错位且**三平台一致地错**——金向量闸门只抓跨平台分歧、抓不了这种"一致地错"（见 CLAUDE.md
"金向量闸门的能力边界"）。Spec 2 拍板令 `EclImage`/`WorldTables` 记录各自的 `content_hash`，
运行期比对拦截。**追注（颜色轴刀，2026-07-26）**：`APPEARANCE_STAR=3` 这个举例已过期——
② 段现是空切片，`consts.rs` 里不再有任何 appearance 符号。同一条 coherence 责任现由
表派生常量 `BULLET_COLOR_STRIDE` 承担（`compile_with_options` 里 `i32::from(t.color_stride)`
直接从绑定的 `WorldTables` 读值），注入面因此变成"①（结构常量）⧺②（表符号词汇，现空）
⧺ 表派生一项"；`BULLET_COLOR_STRIDE` 比旧例子更强——它的值直接来自同一个 `WorldTables`
实例本身，不是"同一个宏调用"这种结构性巧合，`content_hash` coherence 守卫覆盖的正是
这条穿线关系换表就换值。

**C11 已焊死**（2026-07-21）：`compile_for_table` 把绑定的 `WorldTables.content_hash` 盖进
编译产物 `EclImage.content_hash`；`start_main` 拿它与 `World.tables_hash` 比对，不符即拒绝
（`TaskStartError::TableImageMismatch`）。见 follow-ups C11 条。

**仍开放**：T1 复审四 Minor（parser 无递归深度护栏/链式比较左结合未钉/`@70000` wait
越界测试缺/`i32::MIN` 字面量不可拼写）——与常量注入无关，未在本刀清。

### C17. 桥壳四处打磨(桥刀终审分诊,2026-07-24;整局流程刀追一项,2026-07-25)

**已还**(2026-07-25 前置债务刀):②`save_state()` 未开局分支已补 `warn_once`(`W_NO_GAME`),
与 `load_state` 对称,GDScript 侧现可靠 warning 区分"空存档/未开局";③`hud_boss` 已改走
`g.world.view().boss_ui()`(`WorldView::boss_ui()`),与 `hud_player`/`hud_spell` 读口一致。

`bridge.rs`:①`ping()` 测试助手留在生产冻结面(无害欠整洁);④`new_game_at`(bridge.rs)的
loadout 参数是 `character`/`power`/`lives`/`bombs` 四个平铺标量,非 Dictionary——v1 装备维度
少(四件)尚可承受,将来若长出更多装备维度(如子机类型/初始道具)时考虑 Dictionary 化。下次动
壳时与①一并顺手打磨。

### C20. 数学核小件三包（2026-07-23 系统审阅分诊）

① `Angle` 缺 `FULL_TURN`/全圆 raw 具名常量——外接层做 BAM→弧度换算得硬编 65536（`Fx::ONE`
有对称物，`Angle` 没有）；② `Angle` 派生的 `Ord`/`PartialOrd` 是线性序非环形序，现无用点，
但谁用 `a < b` 表达"更接近"就踩坑（65535 与 0 线性远环上近）；③ `world/motion.rs` 的
`isqrt(len_sq) as i32 → Fx` 窄化缺 debug 护栏（`Fx::mul`/`div` 同款坑都有 debug_assert，
唯此处裸奔;正常速度不可达,速度分量 ≥~23000px/帧才触发）。三件都一行级，路过 math/motion 顺手。

### C21. 池布局文档账目过期（2026-07-23 系统审阅分诊）

`docs/pool-memory-layout.md` 弹池汇总行按 19 字段全 4B 估（~625KB），实际近半字段 u8/u16，
精确 ≈433KB（虚高 ~30%；四热字段各 32KB 的 L1 论证不受影响）；`stg-world-design.md` D5
"~64B/敌 16KB" 实为 ~74B/敌 ≈18.5KB（`enemy.rs` 模块注释已自行改口 ~70B）。重算续表即可。

### C22. `tables.rs::validate` 的 ② join 校验循环现空转、无测试覆盖（颜色轴刀 T4 清空 ② 段的残余，2026-07-26）

`WorldTables::validate()` 里 `for c in crate::consts::TABLE_SYMBOLS { if (c.value as usize)
>= self.appearances.len() { return false; } }`（`tables.rs:316-320`）是 C14 记的那道
"② 表符号必须落在 appearances 合法行内"的 FM1 防线；颜色轴刀把 `consts.rs` 的 ② 段清空
（弹型名归内容包，见 C14/C11 追注）后，`TABLE_SYMBOLS` 是空切片，这个循环体永远不执行——
机制还在，但当前不是一条活防护。原有两条覆盖它的测试
（`validate_rejects_table_symbol_without_appearance_row`/
`builtin_appearances_exactly_cover_table_symbols`）已随 ② 段清空一并删除（`tables.rs`
`mod tests` 里留了说明注释）。复审判定**保留循环本身可接受**——② 段将来重新长出行（如
道具类型符号）时机制自动生效，删掉它反而是白扔一次未来要重写的代码——但记档防止将来
复审者看到"零覆盖的校验逻辑"误判为遗留 bug 想删掉它。

---

## D. 设计层面的已知裂缝

### D2. 设计与代码的名字漂移：`frame_events` vs `events`

`stg-world-design.md` 通篇（16 处）+ `design_doc.md`（2 处）+ CLAUDE.md 的 P6 都叫 **`frame_events`**；
**代码里的字段是 `events`**（`WorldBody.events` / `events_len` / `EVENTS_CAP` / `push_event` /
`diag.events_overflow`）。拿设计文档去 grep `frame_events`，代码里**一个都搜不到**。

`hits` 两边一致；`reqs` 已落地且两边同名（通道 B 刀，2026-07-23——设计/代码都叫 `reqs`，
本条漂移仅剩 `frame_events`↔`events` 一处）。

**这不只是审美**：A5 那张表刻意用 `hits`（碰撞命中缓冲）对 `frame_events`（世界大事记）来区分两条
缓冲，`frame_events` 里的 "frame" 正是它的生命周期语义。代码的 `events` 丢了这个区分度。

**修法二选一**（都便宜，但要选一个）：
- 代码 `events` → `frame_events`：纯重命名，`events` 本就 checksum-skip，**金向量校验和零影响**。
  波及 `world.rs`/`world/settle.rs`/`world/player.rs` + harness 探针（无）。
- 或反过来把设计改口径为 `events`（但会丢掉与 `hits` 的对照度，不推荐）。

### D3. 金向量导演的补敌逻辑是计数式补位

`main.rs` 的 `slots.iter().skip(alive)` 只按存活**数量**从头跳，不看哪个槽空。幸存者非首槽时
会把新敌人叠在幸存者身上、同时留一个永久空位。**确定且更病态**（两敌重叠更能压碰撞路径），
诊断场景可接受 —— 但若这个场景被复用到位置敏感的用途，先修这里。

### D7. 池 generation u16 回绕的理论 ABA（2026-07-23 系统审阅新记）

`define_pool!` 的 `generation` 是 u16、`first_free()` 恒取最低空位——池内最热槽必是低索引，
65536 次复用后回绕，**跨帧长期持有句柄**的外部消费者理论上可撞句柄别名（旧句柄"复活"指向
新实体）。全仓现无触发路径（模拟内句柄短命,`Handle::NULL` 哨兵不依赖全零),但 M5 headless
高频 churn 长跑（RL 训练百万帧级）量级上够得到。**触发点**：M5 开工前过一遍 churn 估算，
必要时 gen 扩 u32（池账 +2B/槽）或文档写死"外部句柄不得跨 >N 帧持有"。
（知会，原 D6 尾注，2026-07-25 前置债务刀随 D6 收口挪存于此：`TaskPool` 的 `impl Default`
仍可外部构造空池——但 `tasks` 字段已封,无注入路径,终审 2026-07-23 判无动作必要,记此防将来
误判为漏网。）

### D9. "敌主协程返回即自燃"——设计承诺代码查无实现（场景刀 T7 探查，2026-07-26）

`world.rs:106` 与 `world/cleanup.rs:6` 的注释均写"M1 起敌人主协程返回即自燃——ZUN ECL
语义"（`ENEMY_OOB_MARGIN` 只是防泄漏的大边界兜底，回收"主导"本该靠这条纪律），
`stg-world-design.md` 也有同句。全仓 `grep ENEMY_DYING`：唯一置位点是
`world/settle.rs::hp_break`（伤害血线路径）——**没有任何代码路径在 ECL 任务（尤其
`main_task` 绑定的敌主任务）自然 `return`/结束时把 owner 敌标记 `ENEMY_DYING`**。这条
"任务亡→敌燃"的承诺（注意方向，与既有的"敌死→任务被 owner-gate 清杀"是相反方向、两者都
该成立但现在只有后者是真的）从 M1 起就只停留在注释里。demo 局的杂兵（`stage1.ecl`
`zako_dive`）目前靠退场目标 `y=760`（超过 `FIELD_HEIGHT+ENEMY_OOB_MARGIN=704` 的回收线）
让越界回收兜底顶上，是第一个真实撞上这条空缺的消费者——如果不特意把退场终点设过界，
`zako_dive` 任务 `wait(600)` 结束后敌会**留在场上不消失**。**触发点 = 下一次编排"敌任务
跑完就该退场"的内容且不方便靠越界收尾时**（比如原地驻守型敌、场内消失型敌）。

### D10. 部分设运行期只护 stride、不查 `valid`——编译期空格闸只覆盖 `.ecl` 源码路径（颜色轴刀 T6 记档，2026-07-26）

`OP_SET_SPRITE`/`OP_SET_SHAPE`/`OP_SET_COLOR`（`world/transform.rs::fire_op`）三个解释臂
只做一条运行期护栏——stride/参数是否会导致除零或溢出（P4-b：坏参数计 `contract_viol` 后
no-op；算术上 `wrapping_add` 防近 `i32::MAX` panic）——**从不索引
`WorldTables.appearances[..].valid`**。空格闸（拒收"落到图集空格的组合"）只活在编译器
前端（`lang::atlas::{check_shape_color, check_shape_only, check_color_only}`，挂在
`lang::codegen` 的 `OpFold2`/`OpWithStride` staging 阶段）。这对 `.ecl` 源码路径足够——
一切 `fire`/`batch`/`set_sprite`/`set_shape`/`set_color` 调用都先过这条编译期闸——但
**任何绕开 `lang::compile` 直接构造/反序列化 `XformSlot` 的路径都不受这条闸保护**。目前
全仓没有这样的路径（唯一产出 `XformSlot` 的是编译器 codegen 与手写测试助手），所以不是
活漏洞，只是"编译期闸的覆盖面比看起来窄"这条事实需要记档，防止将来有人以为空格在任何
路径下都造不出来。**触发点 = 出现直接构造/反序列化 `XformSlot` 的消费者时**（候选：
mod 提供的二进制 xform 段格式、M4 rollback 对端镜像重放）——届时需要在运行期臂补一条
`valid` 校验，或明确记录"信任构造方已经过编译期闸"这条前提由谁来担保。

---

## E. bomb 那一刀开工前

- **`world/player.rs` 的 `update_players`** 里，`LIFE_DEATHWINDOW` 臂有一句
  `// bomb 救人 stub：本切片无 bomb 输入 → 窗口必耗尽。` —— 那是 M0-7 留的挂点。
- **「被消弹区清掉的弹还算不算 graze？」已答：算**（擦在相位 6 已发生、清弹是相位 7 的事；
  设计明写 graze 独立于中弹）。理由与推导记在 `world/settle.rs` 趟三的注释里 + M0-8 spec。
- bomb 是 `FieldPool` 的**首个真租户**（消弹区已就位，bomb 只需铺一个 `FIELD_CLEAR_BULLETS |
  FIELD_DAMAGE` 的 field）。

---

## F. 长期预留（M0-18 性能审记档，均不动现刀）

### F1. 嵌入式画像三条（单片机移植预留）

基线（`docs/bench-baseline.md`）实证：CPU 不是墙（Cortex-M7 级估 20-40× 慢，真实规模仍 <1ms/帧；
I1 定点+查表+整数 CORDIC 本就是无 FPU 生存姿势，烘焙表天然走 flash），**RAM 才是**
（World 0.92MB > 多数 MCU SRAM）。预留三条：
① **池容量缩编档**：cap 全是 `define_pool!` 编译期常量——弹 2048/xform 512/自机弹 256 档
   估 World ~250-300KB，进 Teensy/ESP32 级；将来做 feature/profile 化，不动架构；
② **`no_std` 触点清单**：stg-core 零外部依赖（防火墙既定），全 crate 唯一 std 触点 =
   `step.rs::World::new` 的堆分配（MCU 上换 static 分配，feature gate）——**新增代码别引入
   新 std 触点**（纪律）；
③ CI aarch64 对拍已证 ARM 整数语义；Cortex-M 同族，金向量上 MCU 对拍是将来最硬的确定性招牌。

### F2. 校验和轻量化——**已裁决:保 FNV 冻结(2026-07-23)**

**裁决**（存档字节格式/恢复重演风暴闸刀，2026-07-23）：保 FNV-1a64，不换。依据——校验和
无任何**在线逐帧消费者**（单机不算、联机 K=20 采样摊薄 ~70µs(bench 实测 1414µs/20;新账 1.56ms/20≈78µs,A2 续表时以重跑为准)、CI/storm 皆离线跑）；换字宽
mix 只省离线工具的运行耐心，却要为此重新 bless 金向量、多背一刀改动。`ENGINE_VER = 1` 的
身份语义**即含 FNV-1a64**这一算法选择——真要换算法，走 bump `ENGINE_VER` + 过评审的正规
流程，不是本刀的份内事。活口：若 M4 网络实测采样成本超预算，再重启本条讨论。以下原分析
正文保留作裁决依据。

FNV-1a 64 逐字节扫 0.92MB ≈ 1.3-2ms/帧（基线实测最大单项，step 本体的 10-20×）。运行时
本可回避（单机不算、联机 K=20 采样摊薄 ~70µs），故**不紧急**；但若换，窗口在 **M3 回放头/
握手把算法身份冻结之前**（现在换 = 三平台流一起变、零存量破坏；之后换 = bump engine_ver）。
候选：字宽化 mix（wyhash/rapidhash 式 8-16B/步乘法折叠，vendored ~50-80 行，预估 ~10×
到 100-200µs）> vendored xxh64 > 硬件 CRC32（平台可用性参差，仅 32 位）。实现注意：
`#[derive(Checksum)]` 走流式 hasher，字宽化需内部 8B 缓冲；数组元素逐个 `write(4B)` 的
调用开销也在账里，可为 `[i32; N]` 走批量字节路径。

**采样化讨论结论（2026-07-18 grill）**："随机校验"两台机必须采同字节才可比 → 健全形态 =
**确定性分块轮转**（帧号定块，1/16 每帧，16 帧全覆盖，~130µs/帧）。分歧两型：级联型任意
采样 1-2 帧即抓；**休眠型（`facing`/`sprite` 等纯表现字段，P6 特意入哈希）不级联**，检测
延迟 = 覆盖周期。对比既定 K=20 全量采样（摊薄 ~100µs、延迟 ≤20 帧）增益甚微——**不立项**，
仅作"将来要更低分歧延迟"的备选记档。CI/金向量保全量逐帧不动（"首个分歧帧"定位是手术刀）；
版本/mod 不符由 A3 内容哈希在握手拦，不属状态校验和职责。

### F3. 查看器泛化（serve 的将来扩展点，2026-07-23 记档）

v1 只跑彩虹风铃卡固定场景。两个自然延伸，各随触发点：`--ecl <path>` 任意脚本预览
（触发 = .ecl 创作流真开动——写卡即看，编译错误回显进页面）；回放文件播放/逐帧步进
（触发 = M3 回放调试，线格式 v1 直接可复用为 dump 格式）。多客户端/TLS/断线续联不做
（测试工具本分）。

### F4. 逐帧全缓冲上传账未记（场景刀 T7 记档，2026-07-26）

`bridge.rs::step_frame` 对每个已注册层无条件 `multimesh_set_buffer` 整块上传（不做脏
检测/增量），四层容量 `bullets=8192, shots=1024, enemies=256, items=512`（`playfield.gd`
`CAPS`）× `FLOATS_PER_INSTANCE=12` × 4B/float = 9984×12×4 = **479232 B = 468 KB 整/帧**，
60Hz 下 ≈ **27.4 MiB/s** 恒定上传带宽（与场上实际实体数无关，哪怕场上空场也全量推满 cap
大小的零缓冲）。`docs/bench-baseline.md` 目前完全没有这条账——它记的是 step/快照/校验和曲线，
不含 M2 渲染链的桥面开销。**触发点 = M2 收口后**（本刀 DoD 是"可玩+可验证"，不含性能
调优）：`encode_layer`（`frame.rs`）本身已把活槽压到缓冲**前缀**（`iter_alive()` 只推进
`n`，尾部留空），`multimesh_set_visible_instances(rid, n)` 也已按实际活数设——但
`multimesh_set_buffer` 上传的仍是**整个 `cap` 长度**的 `Vec`（含 `n` 之后全是零/陈旧的
尾部），带宽账按 `cap` 算而非按 `n` 算。真到了要优化的时候，方向是按 `n*FLOATS_PER_INSTANCE`
切片上传（只送前缀）或脏检测（层内容与上一帧逐位相同则跳过 `set_buffer`）。
