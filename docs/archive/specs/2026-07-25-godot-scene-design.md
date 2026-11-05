# Godot 场景刀设计(渲染契约收口 + 真工程竖切)

> 2026-07-25 brainstorm 拍板。M2 后半程:桥刀(2026-07-24)交付了 cdylib + 15 口冻结面,
> 本刀交付**真 Godot 工程**——第一次在 Godot 里看见并打完风铃卡。
> 前置裁决已入册:follow-ups A5 初裁(乙案单干)+ 续裁(符卡练习 select 定式,5c5cd30)。

## 1. 目标与 DoD

**竖切:打风铃卡。** `godot --path godot/` 开局即玩一整段 demo 局:杂兵段 → 风铃卡
boss 战 → 挂牌结算,键盘操作,四层弹幕可见,HUD 全须全尾,敌死有爆点,符卡有宣言横幅,
BGM/BG 锚点有占位演出。

DoD 三判据:
1. **可玩**:真 Godot 工程里 demo 局全程可玩可打,风铃卡取得/失败两条路都走得通;
2. **headless 冒烟**:真工程级开机冒烟(非桥级)跑 N 帧断言过闸,run-smoke 同构脚本绿;
3. **cargo 全绿**:A5 乙案判别式测试齐;金向量允许**一次性整体平移**(编译镜像因
   `spawn_enemy` 追参必变,判别式测试护行为——锚点字段平移先例 8676d06)。

**用户拍板记录**:范围=竖切(A5 折进本刀)/画面=经典 640×480/美术=程序化占位
(用户稍后自制真美术,契约钉死后只换 PNG)。

## 2. 范围裁定(做/不做)

| 做 | 不做(YAGNI 挡板,各有归宿) |
|---|---|
| A5 乙案(`spawn_enemy` 追 task 参 + `main_task` 通电) | A4 背景 mini-VM(`bg_phase` 占位硬编码相位表) |
| 四层 MultiMesh + 图集 shader + 占位图集管线 | 标题菜单/选关 UI(practice 走 `start` 参数,无 UI) |
| 请求分发器(引擎 7 id 全 handler + 64+ 注册口) | 音频资产(BGM=HUD 曲名标签+日志,占位) |
| HUD 右栏 + boss 血条/符卡名/计时 | replay/联机 UI、手柄、显示插值 |
| 挂牌结算 overlay + Esc 暂停 | `death_script` 字段通电(与 main_task 不同,无消费者) |
| demo 局多文件 .ecl(吃 compile_units/mark/锚点) | 敌机朝向/动画状态机(`anm_state` 不消费) |
| B18 余量捎带(hud_spell/fields_info 判别断言,A5 解锁) | 桥面扩口(现 15 口够竖切,零新增) |

## 3. A5 乙案:`spawn_enemy` 追 task 参(断层线下唯一改动)

**表层签名** `spawn_enemy(x:Fx, y:Fx, hp:Int, drop_table:Int, score:Int, task)`——
第 6 参与 `fire` 第 7 参**同构**:编译期解析的标识符(async 无参 sub 名)或字面量
`none`,builtins.rs 参数位类型 `Sub`(`fire`/`spell_begin` 第三位既有机制,零新语法)。

**VM 语义**(`sys_spawn_enemy`,镜像 `sys_fire` 的 task 路径,口径逐条同):
1. `task_script` 先验后建:`>=0` 时查号在册 + `SubKind::Async` + 零参,坏号
   `FAULT_BAD_OP`(**敌未建**);`-1`(`none`)= 无任务;
2. 建敌成功后 spawn 任务:`owner = (OWNER_ENEMY, handle.index, handle.generation)`,
   `parent = ctx.self_index + 1`,出生帧 = 当帧(出生当帧不跑,门禁既有);
3. 任务池满 → 静默计数 `diag.pool_full[POOL_TASK]`(P4-a,敌已建、句柄已押,不 Fault);
4. **`main_task` 通电**:回填 `enemies.main_task[idx] = 任务槽号 + 1`(0=无任务/池满
   降级)。纯记账无行为,消费者=将来死亡演出/调试;P6 自动入校验和,无新字段、快照尺寸不变。

**由此解锁**(owner-liveness gate 与 owner 门禁全部既有,零改动):enemy-owned 任务里
`spell_begin`/`move_to`/`self_*` 过闸;敌死/换代 → 任务下相位被杀;符卡任务经
`spell_bound+epoch` 随卡生死。纯 .ecl 从此摆得出 boss。

**迁移面**:全仓 .ecl 调用点追 `, none`(golden rainbow/桥冒烟/frame.rs 测试 SRC/
docs 例子);`gen-ecl-meta` 重跑(JSON/VS Code 扩展/ecl-lang.md 三 sink);金向量整体
平移一次(理由入 commit)。

**判别式测试**(行为正确性只能靠单测守,金向量守不了):
- 带 task 造敌 → 任务里 `spell_begin` 成功(owner=enemy 过闸,旧态必 Fault);
- 敌死 → 下一相位任务被 owner-gate 杀(存活任务数判别);
- `none` → 无任务、`main_task==0`;坏号 → Fault 且敌未建(池计数判别);
- 任务池满 → 敌活、计数增、`main_task==0`。

## 4. 渲染契约收口(sprite 语义,最后一块板)

**契约**:引擎只发 u16 sprite 号;"号 → 图"归表现层。落法:

- **每层独立图集 + 独立 id 空间**(bullets/shots/enemies/items 四张 PNG)——bullets 的
  号来自 `appearances[].sprite`、items 来自 `item_cfg[].sprite`、enemies 来自脚本参数,
  三个号域本就互不相干,分层图集免冲突,也是给美术的最自然交付单元。
- **uniform 网格,sprite 号 = 格号(行优先)**:cell 尺寸 bullets/shots/items 32×32、
  enemies 64×64;shader uniform 给 `(cell_px, cols)`,QuadMesh 尺寸 = cell 尺寸
  (1px = 1 world unit,场界坐标系即像素)。
- **shader**(`layer.gdshader`,canvas_item):vertex 读 `INSTANCE_CUSTOM.x` 得格号
  → 算 UV 偏移;`custom.y/z/w` **保留空位**(将来 scale/alpha/调色,stride 12 不变)。
- **占位图集**:`tools/gen_atlas.gd` 一次性确定性绘制(米弹/圆弹/星弹按号区分色形,
  敌=色块,道具=图标),产物 commit——同烘焙表纪律,再生成可对拍。真美术接手只换 PNG,
  网格契约不动。自机/判定点两张独立小图同产。
- **契约文档** `docs/render-contract.md` 新开,收口全部表现层契约:12-float 布局/
  每层图集网格/id 空间/引擎 7 id 分发表(指 reqs.rs 权威)/锚点双表示规矩/坐标映射。
  首要读者=美术(用户)与壳作者;CLAUDE.md 结构树补行。

## 5. 仓库布局

```
godot/                       ← 真 Godot 工程(仓库根,非 crate)
  project.godot              640×480,canvas_items 拉伸+整数倍缩放,physics 60Hz
  stg_godot.gdextension      指 ../target/{debug,release}/libstg_godot.so
  scenes/   main.tscn(+hud/effects 子场景按需)
  scripts/  main.gd input.gd dispatcher.gd hud.gd player.gd bg.gd content_tables.gd
  shaders/  layer.gdshader
  assets/   bullets.png shots.png enemies.png items.png player.png hitbox.png(占位)
  tools/    gen_atlas.gd
  ecl/demo/ main.ecl stage1.ecl boss_windchime.ecl
  smoke/    run-smoke.sh(真工程级冒烟)
crates/stg-godot/smoke/      ← 桥级冒烟不动,继续守桥面回归(A5 后加符卡两断言)
```

`.uid` sidecar 随源入库、`.godot/` 忽略(坑档 G2);`--import` 首跑非致命处理(G1)。

## 6. 场景树与每帧回路

```
Main(main.gd 状态机: PLAYING / PAUSED / STAGE_CLEAR / GAME_OVER)
├─ WorldBridge
├─ PlayfieldContainer(位置 32,16)
│   └─ SubViewport 384×448          ← 场界即视口;内容根节点位置 (192,0),
│       │                              世界坐标直接当本地坐标(x∈[-192,192],y∈[0,448])
│       ├─ Background(bg.gd)
│       ├─ ShotsLayer → EnemiesLayer → ItemsLayer(MultiMeshInstance2D,z 序即节点序)
│       ├─ Player(Sprite2D,吃 player_pos();按住 SLOW 叠显判定点)
│       ├─ BulletsLayer(敌弹压最上,东方惯例)
│       └─ Effects(一次性演出:爆点/飘分/横幅)
├─ HUD(CanvasLayer:右栏分/残机/bomb/power/graze;boss 血条绝对定位覆盖弹幕域顶部)
└─ Dispatcher
```

**每帧回路**(`_physics_process`,Godot 自带 60Hz 定拍——坑档 C2 的补帧策略是裸循环
宿主的事,Godot physics tick 自管追帧,不自造节拍器):
1. `input.gd`:InputMap 动作(`stg_up/down/left/right/shot/bomb/slow`)拼位掩码
   (`BTN_*` 桥常量) → `bridge.step_frame(mask)`(桥内已含编码+上传);
2. `dispatcher.drain(bridge.take_requests())`;
3. `hud.refresh()`(轮询 `hud_player`/`hud_boss(0)`/`hud_spell(0)`);
   `player.position = bridge.player_pos()`;
4. 状态机不在 PLAYING 时**不调 step_frame**(宿主暂停 = 世界时间线零帧;不用 Godot
   pause 树——演出/overlay 在暂停里继续动是特性不是 bug)。

**开局/读档对表规矩**(锚点双表示,硬规矩):`new_game_at`/`load_state` 成功后必须
一次性读 `anchors()` 对齐 BGM/BG/相位——事件(REQ_*)走分发是增量,电平(锚点)是
真值;中段开机垫片补偿只保证字段+一发请求,错过的请求靠电平追平。

## 7. 请求分发器契约

`dispatcher.gd`:`handlers: Dictionary`(id → Callable),未知 id 打一次去重日志。
引擎 7 id(`reqs.rs` 权威表)内置注册:

| id | handler 占位演出 |
|---|---|
| `REQ_ENEMY_DEATH` | 爆点粒子(x/y raw÷65536 转 px)+ 飘分(args[3]) |
| `REQ_SPELL_DECLARE` | 符卡宣言横幅(名查 `content_tables.gd` id→名字典,缺省 "Spell #N") |
| `REQ_SPELL_RESULT` | 取得/失败横幅 + bonus 飘字 |
| `REQ_STAGE_CLEAR` | 转 main.gd:停拍 → 结算 overlay(分数来自 hud_player)→ 按键续行 |
| `REQ_BGM` | HUD 曲名标签(`content_tables.gd` id→曲名)+ 日志(无音频资产) |
| `REQ_BG` | bg.gd 换纹理/底色 |
| `REQ_BG_PHASE` | bg.gd 硬编码相位表(0=滚动/1=停,A4 落地后由 mini-VM 接管) |

64+ 脚本段:`dispatcher.register(id, callable)` 公开口,demo 内容自注册(演示惯例)。
名表(曲名/符卡名)归**内容包**(`content_tables.gd`),不进引擎——id→名是表现层契约。

## 8. demo 内容(.ecl 多文件,整局流程刀机制的第一个真实消费者)

- `main.ecl`:`bgm(1); bg(1);` → `mark(1)` 杂兵段(call stage1 的编排)→
  `mark(2) { }` boss 段(垫片自动补偿 bgm/bg;boss 战 BGM 手写 emit)→ 风铃卡后
  `emit_req(REQ_STAGE_CLEAR, ...)` 挂牌 → 局终。
- `stage1.ecl`:两三波杂兵(`spawn_enemy` 带/不带 task 各有),掉 power/point。
- `boss_windchime.ecl`:`spawn_enemy(..., boss_main)`;`boss_main`(enemy-owned):
  `boss_set` 公告板 → 非符段 → `spell_begin` 风铃卡(从 harness `scenes/rainbow.ecl`
  移植)→ `wait_spell` → 死亡掉落。**同时是 A5 的狗粮**。
- practice 留口:`mark` 位就绪,`start` 参数从冒烟走通(headless 冒烟用 `start≠0`
  验证真工程链路的中段开机),UI 后补——符卡练习定式见 follow-ups A5 续裁。

## 9. 冒烟与测试

- **cargo**:§3 判别式测试 + 既有全绿;金向量平移一次入册;`gen-ecl-meta` 产物同步。
- **桥级冒烟**(既有,扩两断言销 B18 余量):`godot_smoke.ecl` 加带 task 的
  `spawn_enemy` + `spell_begin` → `hud_spell` 非默认判别断言 + `fields_info` 非空
  (符卡清弹 field 可达)。
- **真工程冒烟**(新):`main.gd` 识别 `--smoke` 用户参数(`OS.get_cmdline_user_args`)
  → **两次开机**:① `start=0` 正常开局跑 N 帧,断言 checksum 非零/收到过 REQ_BGM/
  player_pos 在场界;② `start=2`(boss 位 mark)中段开机,断言 `anchors()` 电平已被
  垫片补偿追平(bgm/bg 非 0)→ `SMOKE OK` + 退出码。`godot/smoke/run-smoke.sh`:`--import` 首跑
  `|| true`(G1),**不许 `set -e` + 命令替换吞诊断**(follow-ups B22 教训,新脚本
  直接写对,顺手把 B22 老脚本也修了销账)。
- CI 不动:Godot 宿主本机专属,冒烟是本地闸(与桥刀口径一致);cargo 面照常三平台。

## 10. plan 钉子(任务切分预览,writing-plans 细化)

1. **T1 A5 乙案**:builtins/codegen/sys_spawn_enemy/main_task 通电 + 判别式测试 +
   全仓 .ecl 追参 + gen-ecl-meta + 金向量收口(平移入册);
2. **T2 契约+图集**:`docs/render-contract.md` + `tools/gen_atlas.gd` + 六 PNG commit;
3. **T3 工程骨架**:project.godot/gdextension/main.tscn/input.gd/节拍/状态机壳 +
   `--smoke` 通路(空局);
4. **T4 渲染链**:四层 MultiMesh + layer.gdshader + player/bg 节点(桥冒烟脚本内容即可见);
5. **T5 分发器+HUD+演出**:dispatcher/hud/effects/结算 overlay/暂停;
6. **T6 demo 内容**:三文件 .ecl + 桥级冒烟两断言(B18 销)+ 真工程冒烟收口 + B22 修;
7. **T7 文档收口**:CLAUDE.md 树/PROGRESS/follow-ups(A5 销、B18 销余量、B16①③④
   触发点复核)/bridge-adaptation-notes 新坑追加。

依赖链:T1 独立先行(纯 Rust);T2 独立;T3→T4→T5 串行;T6 吃 T1+T5;T7 收尾。
