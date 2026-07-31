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
>
> 最后核实：2026-07-27（P4 覆盖刀销账：逐条核实后整条删除 **B1**〔四个 `create_*` 池满
> 测试〕/**B2**〔`push_event` 溢出测试〕/**B4**〔overkill hp 断言〕/**B11**〔越界
> `drop_table` 测试〕/**B12**〔`credit_item` `saturating_add`+`>=` 修复与测试〕/**B14**
> 〔`storm --saves 0` 守卫+去重+测试〕/**B25**〔`spawn_enemy`/`fire` task 号支路①③四条
> 测试，`main_task` doc 注释已随该刀补在 `enemy.rs`〕，七条均已在代码里核实落地）。
>
> 最后核实：2026-07-30（**ECL 复刻刀**销账：逐条核实后整条删除 **D9**〔`vm::run_tasks` 的
> `Exec::End` 分支已自燃 + 四条测试 + `enemy.rs`/`world.rs`/`cleanup.rs` 三处注释改口径 +
> demo 杂兵退场点改回场内〕/**B19**〔`SYS_CLEAR_BULLETS`=54 + 表层 `clear_bullets()` +
> 两条测试；语义拍板"直接消 + 消弹转星星"，不做护盾帧——那归 bomb〕/**B20**〔55/56/57 三个
> syscall + `add_lives`/`add_bombs`/`add_power` 三个内建 + 四条测试；形态拍板为**增量**
> `add_*`，`set_*` 不做，绝对赋值归 `Loadout`〕。三条的语义均已写进 `docs/ecl-lang.md`
> 手写节与 `docs/ecl-ops.md` 号表。）
>
> shooter 刀收口追记（2026-07-31）：新记两条，均为**本刀有意不做/有意不对齐**的记档，不是
> 缺陷单——**D13**（ZUN 随机 aimmode `6`/`7`/`8` 不做：消耗世界 RNG，抽取发数与顺序直接进
> 校验和，要单独一份 spec 钉死）、**D14**（`sh_fire` 与 `create_bullets_batch` 的两处口径
> 差异：弹池满不做剩余批量补计 / 退化网格与 appearance 的校验先后序相反——**两处都影响
> 进校验和的值，别"顺手对齐"**）。本刀顺带删掉 `builtins::folds_shape_color`（两个真消费者
> 已转投 `fold_start`，它只剩测试在调且答的是没用的那一半）。
>
> shooter 刀**整支复审修复波**再追一条（2026-07-31）：**C23**（`SHOOTERS_PER_TASK` 没作为
> C14 引擎常量注入 ⇒ 手册与所有脚本硬编码 `0..=3`，而它每个兄弟都是注入的）。同波顺带
> 订正、**不入清单**（已改完，不留墓碑）的四处：`ecl/task.rs` 模块文档的容量数字过时 ·
> `bench-baseline.md` 文件头"贴在旧表上方"与实际的向下追加自相矛盾 ·
> `ecl-ops.md`/`ecl-lang.md`/`xform-ops.md` 三处漏枚举 `sh_fire` 的 **xform 段池**压力 ·
> `ecl-ops.md` 75 号补记 `sh_req` 为何与 `emit_req` 收窄口径不同（钳 vs no-op）。
>
> 敌人死亡效果刀 T1 追记（2026-07-30）：新记 **D11**——掉落从"生成时定死的表索引"迁成
> "敌身上按类型计数的可变状态"后，掉落表的**条目顺序不再影响任何东西**（撒的顺序由
> `spill_drops` 的类型升序决定），而 `validate()` 不要求条目升序 ⇒ 非升序的内容包表会
> **静默**改变 RNG 消耗顺序，无任何检查会红。

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

### B3. 多自机 graze 位隔离 —— 被 co-op 阻塞

`grazed_by` 是每自机一位的掩码。「P0 擦到的弹不该置 P1 的位」目前无测试，因为
`players[1]` 在所有场景里恒为 `LIFE_ABSENT`。**需 co-op 出场机制才可测**，届时一并做。

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

### B13. 道具近距磁吸 v0 的自机选择与优先裁决序偏离 spec —— 与 B3/B8 co-op 族同批（M0-12 终审分诊）

道具近距磁吸 v0 实现取"升序首个圈内自机"，而非 spec 写的"最近的 ALIVE 自机"——`players[1]`
恒 `LIFE_ABSENT`，两种取法在单机场景下不可观测、无法用当前测试区分。另外 PoC 磁吸与近距圈
两者的优先裁决序 spec 未写明，也是同一刀留下的空白。co-op 出场机制到位（解除 B3/B8 阻塞）后
与它们一并定案；此处先把"spec 偏离"显式记档，避免后续误当 bug 修掉。

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

### B23. ~~`layer.gdshader` 图集选格 UV 垂直朝向嫌疑~~ —— **已判决：不镜像，无需修改**（2026-07-27）

**判决结论**：真美术上屏后画面正常，UV 朝向没有问题，`layer.gdshader::fragment()` 的
`uv = (cell + UV) / vec2(grid_cols, grid_rows)` 保持原样，**候选修法（`1.0 - UV.y`）不要采纳**。

**证据**：颜色轴刀接入原作弹片（`godot/assets/bullets.png`，12 弹型 × 16 色）后，格内图元
本身就是强上下不对称的——arrowhead 有朝向、kunai 是三角带**下方**小圆、laserhead 是端头
圆帽。作者在有 GPU/X 的环境跑了真工程，画面无异常。若 shader 真把每格上下镜像，这批图元
会立刻显出"苦无的圆跑到上面去了"一类的错位，不可能看不出来。这比原先设计的判决程序
（往占位图集塞一张上红下蓝的测试格）覆盖面更大——整屏所有弹型同时受检。

**留档理由**：原条目记录的静态观测（`QuadMesh.get_mesh_arrays()` 读出的顶点/UV 配对看似
会导致镜像）本身没错，错的是由它外推到"最终采样结果"——中间的 NDC/视口变换/`CanvasItem`
坐标系翻转把它抵消了。**教训**：CPU 侧网格数据不能直接外推 GPU 光栅化结果，这类问题只能
上真渲染器判。

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

### B26. 首个有 GPU/X 环境的三件套合并单（2026-07-26；**2026-07-27 部分兑现，余 ① 一条**）

三条独立记档的债都卡在同一个前提——**本机全程无 GPU/无 X**（`xdpyinfo` 探测失败），
且**全刀无一次有头运行**（画面从未被人眼看过，含本条判决程序涉及的自机 `Sprite2D`/
弹幕图元/HUD 排版）——各自记在各自条目里容易各等各忘，合并列一次防止漏项：

① ~~**B23** UV 垂直朝向~~ —— **已销（2026-07-27）**：真美术上屏画面正常，判定不镜像，
shader 不改。详见上方 B23 条。**追注（颜色轴刀，2026-07-26）**：占位图集本身已经上下不对称
（`_disc_shaded`，颜色轴刀 T5），首个有头启动即可直接肉眼判别、不必再临时加测试格；
判决程序其余步骤（对照 `layer.gdshader` 候选改法）不变。

② **B18** `visible_instances`（`MultiMesh` 可见实例数）不可 headless 断言——判决程序：
有 GPU/真渲染器后重跑 `godot/smoke`，把当前绕开用的 `multimesh_get_buffer` 判据换回
`RenderingServer.multimesh_get_visible_instances` 直接断言，确认非 0（详见 B18 条内
两层不可断言的具体原因）。

③ **DoD 可玩目验（部分兑现 2026-07-27：画面已经人眼看过、无异常；
是否完整打到风铃卡结算、输入手感与 boss 战节奏是否可接受，仍未确认）**——`CLAUDE.md` Phase 1 之外，本刀（Godot 场景刀 + 本次终审修复波）
的隐性 DoD 是"可玩 + 可验证"，但**验证目前全靠 headless 冒烟断言，没有一帧被人眼看过**：
demo 局的图集贴图是否如预期摆放、HUD 排版是否重叠、boss 战节奏是否真的可打（900hp 是
仓外探针实测数据，见 `godot/ecl/demo/boss_windchime.ecl` 注释，不是本机有头试玩验证的）、
输入手感是否正常——一概未经目验。判决程序：有 GPU/X 环境后 `godot --path godot`（非
`--headless`）跑一局 demo，键盘操作到风铃卡结算，肉眼确认贴图/HUD/节奏均正常。

**剩余**：② `visible_instances` 断言要改冒烟脚本后在有头环境重跑；③ 的可玩性部分需要
真打一局到结算。两件仍共享同一次有头启动成本，凑一起做。

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

### B28. 擦弹特效未接——事件化须先定聚合口径（命中事件刀记档，2026-07-27）

自机弹命中已走 `EVT_SHOT_HIT_ENEMY`（逐命中发，见 `render-contract.md` §3.5），**擦弹刻意
没跟着做**：擦弹频率高一个量级（满屏弹幕贴身可以每帧上百次），逐条发必撑爆 `EVENTS_CAP=512`
——与 `EVT_FIELD_CLEARED` 当初被迫聚合是同一个约束。真要做时的选项：①每自机每帧聚合一条
（带本帧擦弹数，表现层自己撒火花）；②不发事件，宿主从 `hud_player` 的 graze 计数做帧间差分
（零引擎改动，但只有计数没有位置）。**触发点 = 真要做擦弹演出时**，届时先定口径再动手。

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
   `last_status`/~~`nearest_enemy`~~/`attract_all_items`（号表 v2 候补），`SPAWN` 的
   "owner 来源枚举操作数"简化为恒继承——现实现更简且够用，记录在案防"悄悄丢"。
   **`nearest_enemy` 已还**（小清洗刀，2026-07-31）：`SYS_NEAREST_ENEMY`(79) + 同名表层
   内建，世界侧 `world::nearest_enemy`（M0-13 起的死代码）一行未改，只是通了电。**剩两项
   仍是候补**。追一条新的小缺口：脚本拿到敌号后**只能喂 `enemy_hp`**——按号读敌坐标的读口
   （`enemy_x`/`enemy_y`）没有暴露，所以"查最近的敌 → 朝它开火"这条链路还接不通；要它得
   再开两个读族号（本刀有意不顺手加，加号得过号表纪律）。

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
现收两参（`shape`/`color`），`rainbow.ecl` 已改写成 `fire(OUTLINE, COLOR_CYAN, ...)`
这类调用，`OUTLINE`/`COLOR_CYAN` 是脚本自己用 `const` 声明的内容包词汇（真美术接入后
词表已换成 LASER/ARROWHEAD/OUTLINE/BALL/… 十二行，见 `godot/ecl/demo/bullets.ecl`），不再是
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
复审者看到"零覆盖的校验逻辑"误判为遗留 bug 想删掉它。**触发点 = ② 段再长出符号**
（如道具类型符号表落地）时，记得给这条 join 校验重新配一对正/负测试，别让它继续裸奔。

### C23. `SHOOTERS_PER_TASK` 没作为 C14 引擎常量注入，脚本只能硬编码 `0..=3`（shooter 刀终审记档，2026-07-31）

`crate::ecl::shooter::SHOOTERS_PER_TASK = 4` 是 `sh_*` 族（syscall 62-76）**槽号 `id` 的
合法上界**，越界走 P4-b（no-op + `contract_viol`，不 Fault）。但它**没有进 `consts.rs` 的
① 结构常量段**，而它的每一个同类兄弟都进了：`GLOBALS_SYS_SEGMENT`（同样是"脚本必须知道的
边界值"）、`REQ_SCRIPT_BASE`、`ITEM_*` 五个；连表派生的 `BULLET_COLOR_STRIDE` 都由
`compile_with_options` 注入。后果是 `docs/ecl-lang.md` 的发射器节与**所有将来的 `.ecl`**
都只能把 `4` / `0..=3` 写成字面量——正是 C14 那条"跨语言常量引用缺失"要消灭的形态。

**为什么本刀没做**（不是遗漏，是范围判断）：K=4 由 D-1 拍死、短期不会动，而注入它要碰
`consts.rs` 的 ① 段 ⇒ 改 `ENGINE_CONSTS` 的内容 ⇒ 所有脚本可见词汇表变化，本该和别的
常量增补一起走一次。**触发点 = 下次动 `consts.rs` ① 段**（或有人真想改 K）时顺手加一行
`SHOOTERS_PER_TASK: usize as int = crate::ecl::shooter::SHOOTERS_PER_TASK;`，同时把
`ecl-lang.md` 那句"编号 `0..=3`"改成引用常量。注意 `engine_consts!` 的 v0 限制是
`$val as i32` 要求原生整数——`usize` 可以，但 ① 段现有各条都是 `u16`/`u8`，加进去时
顺带确认宏的 `@ty` 分支与 `assert` 口径。

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

### D11. 掉落表的条目顺序自此**不再影响任何东西**，但 `validate` 不要求升序（敌人死亡效果刀 T1 复审记档，2026-07-30）

掉落从"敌身上存 `drop_table: u16`、死时查表逐条撒"迁成"敌身上存
`drop_count: [u8; ITEM_TYPE_COUNT]`、死时按**类型升序**撒"（`world::settle::spill_drops`）
之后，`WorldTables.drop_tables` 里某张表的**条目书写顺序**在运行期已被
`tables::drop_counts` 的展开彻底抹掉——它只把每条 `(ty, n)` 累加进对应类型的槽。后果两条：

① **内容作者若以为能靠调整条目顺序控制掉落的产出顺序，那是错的**，写
`[(POINT,1),(POWER,2)]` 与 `[(POWER,2),(POINT,1)]` 得到完全相同的 `drop_count`，撒出来
一律是 POWER 在前。这本身不是 bug——I4 要求的是确定性顺序，不是"作者书写序"——但它是一条
没写在任何地方的**语义变更**。

② 更咬人的是**迁移等价性论证的脚下**：T1 的等价性（掉落内容 + `spawn_drop` 的世界 RNG 消耗
顺序逐字不变）成立，完全是因为内建表 1 恰好是 `[(ITEM_POWER,2),(ITEM_POINT,1)]` 而
`ITEM_POWER=0 < ITEM_POINT=1`——**表序恰好就是类型升序**这个巧合。特征化测试
`enemy_death_drop_sequence_is_pinned`（`world/settle.rs`）押的是**这个具体巧合**，不是一般性
保证：换一张条目非升序的表，撒出的集合仍相同、但逐颗 `(vx, vy)` 会因 RNG 抽取顺序改变而
**静默**不同，而 `validate()`（`tables.rs`，只校验 `ty < ITEM_TYPE_COUNT`）不会红、
金向量闸门只互比三平台也不会红。

**倾向处置：给 `validate()` 加一条"每张掉落表的条目须按 `ty` 严格升序"的校验**（配一条坏表
判别式单测），把上面这个"巧合"升级成表格式的硬契约——比只在文档里写一句更能防住内容包。
代价是内建表 `tables_v0.bin` 恰好已满足，不需要重烘焙。**触发点 = 第一张非内建掉落表出现时**
（C11 资产管线的 owned 表 / mod 内容包），届时若还没加就必须先加。

### D12. 死亡加分硬编码给自机 0——"谁打死谁得分"在联机/多人下不存在（2026-07-30 记档）

`world::settle::kill_enemy` 里那句 `self.players[0].score = ...saturating_add(bonus)` 的
自机号是**写死的 0**。上游拿不到别的号：`damage_enemy(e, dmg, tables)` 的签名里就没有
"是谁打的"——`shots` 池明明有 `owner: u8` 字段，但 settle 趟二的 `ROW_SHOT_ENEMY` 臂只从
命中记录里取了 `damage`，`owner` 一路没往下传；`ROW_FIELD_ENEMY`（消弹区伤害致死）那条更是
连概念上的"击杀者"都要重新定义（field 的属主是谁？bomb 是谁放的？）。

本刀按裁定就取自机 0，与既有 `SYS_ADD_SCORE`（`syscall.rs`，同样硬编码 `players[0]`）同口径，
单人下完全正确。**注意对照**：道具入账 `credit_item(p, ..)` **不**是这样——它带自机号参数，
拾取者是谁就记给谁。所以联机线上要统一的是"击杀分"与"脚本 `add_score`"这两处，不是全仓。**触发点 = co-op / rollback 联机真的上线时**
（M4 那条线，与 B3 多自机 graze 位、B8 并列自机裁决、B13 磁吸自机选择同批）：届时要一次性
决定"击杀归属"的口径——是把 `owner` 从 `hits` 一路穿到 `kill_enemy`，还是干脆改成共享分数。
在那之前别单独修这一处，改一半会让四处记分口彼此不一致。

### D13. 随机 aimmode（ZUN 的 `6`/`7`/`8`）不做——它们消耗世界 RNG，消耗序直接进校验和（shooter 刀 T4 记档，2026-07-31）

ZUN 的 `607 etAim` 是个九值枚举，本刀（D-6）把它塌成 `aimed`/`ring` 两个正交布尔
（`sh_aim`/`sh_ring`，syscall 71/72），**塌得下的只有 `0-5` 那六个**。剩下三个是另一类东西：

| ZUN mode | 语义 | 参数转义 |
|---|---|---|
| `6` | random angles | `ang1`/`ang2` 转义成方向的 max/min |
| `7` | random speeds | `spd1`/`spd2` 转义成速度的 max/min |
| `8` | 两者 | 同上两条一起 |

**为什么不是"加个 flag 位"就完事**：随机散布要抽的是**世界 RNG**（I3：PRNG 状态是 `World`
字段、随快照回滚），于是"每颗弹抽几发、按什么顺序抽"就不再是实现细节，而是**进校验和的
契约**——写错一次，回放/rollback/三平台对拍全部分岔，而金向量闸门只互比三平台、抓不到
（`CLAUDE.md`"金向量闸门的能力边界"）。要钉死的至少三条：① 抽取发生在网格循环的哪一步
（每颗一发？角度与速度各一发？）；② `6`/`7`/`8` 三种模式下的抽取**发数与顺序**互相之间是
什么关系；③ `n_angle`/`n_speed` 与随机的交互（随机模式下 `angle_step` 已被转义成 min，
那"居中"还成不成立）。这是一份独立的小 spec，不该塞进 shooter 刀的尾巴。

且"随机散布"本身与 shooter 的核心价值（**预存参数集**）正交——`rand_range` 已经在手，
今天想要随机弹幕的脚本可以自己 `for` 循环逐颗 `fire`，只是写起来啰嗦。

**将来的形状**（若做）：`flags` 里加 `SH_RAND_ANGLE`/`SH_RAND_SPEED` 两位（`ShooterSlot`
的 `flags: u8` 现只用了低 3 位，有余量；且槽内还有 2 字节尾部 padding，加字段不涨槽宽——
见 `step.rs` 哨兵注释），`angle0/angle_step`、`speed0/speed_step` 在置位时转义成 min/max，
开火循环里逐颗抽。**触发点 = 第一张真需要随机散布的符卡**；届时先补 spec 钉死抽取序，
再动代码，并且要 bump `ENGINE_VER`（`flags` 语义变化 ⇒ 存档载荷解释变化）。

### D14. `sh_fire` 与 `create_bullets_batch` 的两处口径差异——**都影响进校验和的值，别"顺手对齐"**（shooter 刀 T3 复审记档，2026-07-31）

网格发弹在本仓有**两份实现**：`world.rs` 的 `create_bullets_batch`（world 层）与
`ecl/syscall.rs` 的 `sys_sh_fire`（ECL 层）。第二份是必须的——P1 下 world 不知道"任务"
存在，逐颗挂 `task_script` 只能在 ECL 层做。两份实现在**主干上逐位等价**（有等价测试
`shooter_fan_matches_batch_with_centering_compensation` 押运），但有两处**已知**不同：

**① 弹池满时不做"剩余批量补计"。** `batch` 在短路时把剩余额度**批量**记进
`diag.pool_full[POOL_BULLET]`，使计数与"逐颗试"严格等价；`sh_fire` **只短路、不补计**，
故弹池满时它记的是 **1** 而不是"剩余颗数"。这不是偷懒：`batch` 敢批量补计，是因为它在
循环**之前**就把 xform 内容验过一遍（`WorldBody::xform_args_valid`），进循环后唯一的失败
因就是池满；`sh_fire` **够不着那个判据**（`xform_args_valid` 是 `WorldBody` 的私有关联
函数），它的失败因至少两种——池满（记 `pool_full[对应池]`）与坏 xform 内容
（`create_bullet_with_xform` 记 `contract_viol`）——照抄批量补计会把**坏 xform 导致的失败
误计成 `pool_full[POOL_BULLET]`**。

⚠️ **`diag` 的计数值进校验和**（P6，`diag` 无 skip）。所以"为对齐 `batch` 而顺手修好"
会**静默改掉一个入校验和的值**：回放/存档不兼容、三平台仍一致所以金向量闸门照绿。真要
对齐，正确做法是把 `xform_args_valid` 提权成 `pub(crate)` 谓词、循环**前**验一次再批量
补计——那是一处 world API 改动，与 shooter 刀 spec §13"零 world API 改动"冲突，故本刀没做。
**触发点 = 下一次有正当理由改 world API 时**（或有人真的在意两条路径的 `pool_full` 口径
一致），届时连同 `ENGINE_VER` 一起处理。

**② 校验先后序相反。** `sys_sh_fire` 是"退化网格 → appearance"，`sys_create_bullets_batch`
是"appearance → 退化网格"。可观察后果：`n_angle=0`（或 `n_speed=0`）**配一个坏
appearance** 时，`batch` 会 **Fault**（先撞 appearance），`sh_fire` 只 **no-op + 违约计数**
（先撞退化网格短路返回）。两条单独看都符合各自的 P4 处置，只是**同一份坏参数在两条路径上
的结局不同**。实现忠实照抄了计划骨架的顺序，不算违约，但**手册里不能写"与 `batch` 同
口径"**——现在不是。改哪一边都会动金向量（Fault 与 no-op 的世界演化不同），同样要
bump `ENGINE_VER`。

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
