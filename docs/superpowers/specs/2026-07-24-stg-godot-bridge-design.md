# stg-godot 桥接设计(M2 第一刀)——WorldBridge cdylib + 前置 core 小刀

> **状态**:brainstorm 拍板(2026-07-24)。两刀车次:①前置小刀(全 core 侧)②桥刀(纯新增 crate)。
> 权威上位文档:design_doc.md §6(表现层协议)/ §6.4(Godot 侧参考实现);
> 排练情报:`docs/bridge-adaptation-notes.md`(WS 查看器刀,C1-C3 直接复用)。

## 0. 目标与硬约束

**目标**:`crates/stg-godot`(gdext cdylib → `libstg_godot.so`),Godot 只加载不编译——
`.so` 编完放那,Godot 侧开发全程零 cargo;改 Rust 才重编+重启 Godot(低频)。

**已拍板的四个范围决定**:

1. **刀的范围** = 桥 crate 全量 + headless 加载冒烟(最小 `project.godot`/`.gdextension`,
   不算"写 Godot 游戏代码");真游戏场景/UI 是后续"Godot 刀"。
2. **Godot 版本钉 4.6**(gdext 取支持 4.6 的 API level;本机冒烟用
   `/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64`,脚本可经 `GODOT_BIN` 覆盖)。
3. **v1 API 面 = 渲染环 + 存读档**(L1 现成直通);预测未来/时间回溯是后续加法刀(§7)。
4. **前置账先还**:A1/A2/正典 boot 并一把小刀,先于桥分支落地(§2)。

**硬验收(金向量口径;写 plan 时勘误 2026-07-24)**:
- **桥刀**:零 core 改动 → 金向量**逐字节不变**(机器可验证)。
- **前置刀**:A1 换表后金向量**全帧均匀漂移**——`tables_hash` 是入校验和的 World 字段
  (step.rs:31"恒定不分叉,校验无害"),表 `content_hash` 变则每帧校验和变,"逐字节不变"
  对 A1 不成立。验收改为两条:①漂移必须是"**每一帧都变**"的均匀形态(部分帧变 =
  真行为回归,红灯);②**行为不变性判别式测试**——两份仅 item sprite 值不同的 owned 表
  (`content_hash` 同为 0)驱动同种子世界含道具全生命周期演化,逐帧校验和相等。
  boot 下沉与 A2 不动演化:在 A1 漂移后的流上**逐字节不变**。

## 1. 分层判据(本次 brainstorm 的核心裁决,后续刀沿用)

> **参与确定性契约的归 core;碰浮点/时间/表现的,断层线拦死留桥。**

| 项 | 归属 | 理由 |
|---|---|---|
| 正典开局 `World::new_game` | **core(下沉)** | 回放可移植性(L2 重演=同一初始化)+ 联机握手 §7.2"初始状态由双方从同一确定性初始化各自构造"+ C2"别再各写各的";`EclImage` 本就住 core,零新依赖 |
| 道具贴图真相源 | **core 表列**(A1,§2.1) | 单一真相源;池列方案否决,评审记录见 §2.1 |
| 存读档 / `BTN_*` 位布局 / req id 注册表 / 符卡与 boss_ui 数据 / 校验和读口 | core(已在) | 前几刀已铺对,桥侧纯胶水 |
| f32 转换与 MultiMesh 缓冲布局 | **桥,禁下沉** | I1 铁律,连 `Fx::to_f32` 都不准进 core;转换只在 `frame.rs` 一处 |
| 节拍器 / 补帧策略 | 桥(Godot 侧),禁下沉 | I6,core 无时间 |
| 请求分发与演出 | Godot 侧 | "核出请求,壳做演出";分发表 id→Callable 住 GDScript |
| watermark 投机播放去重(§6.3) | 表现侧,M3 的事 | rollback 表现策略,v1 不做 |
| 整数渲染投影 API(`render_atoms`) | **暂不做**(YAGNI) | A9"过滤是消费者义务"+C1 实证裸切片无摩擦;**触发条件**:第三个消费者(stg-py)出现语义 join 重复时再升 |

## 2. 前置小刀(全 core 侧,先落地)

### 2.1 A1:`ItemTypeCfg` 加 `sprite: u16` 表列

**现状缺口**:三个有脸的池里道具唯一没有贴图真相源——弹池有物化列 `sprite: u16`
(发弹时由 appearance id 查 `AppearanceCfg{radius, sprite}` 解出,id 即弃),敌池有脚本
自由指定的 `sprite` 列,道具池只有 `item_type: u8`,`ItemTypeCfg` 六字段全是物理/账本参数。
消费者只能硬编码 `item_type→贴图`,破单一真相源。

**修法**:`ItemTypeCfg` 加 `sprite: u16` 列(5 行各填值)+ WorldTables 序列化格式
version bump + `content_hash` 随之变 + join 校验测试(每行 sprite 有值且互异——判别值纪律)。
**消费端渲染 join** = `tables.item_cfg[item_type].sprite`。

**评审记录(池列方案否决,2026-07-24 brainstorm)**:

- 曾议"照弹池物化一列进 ItemPool"。否决理由:
  1. **键的寿命不同**——弹的 appearance id 创建瞬间死(池不存),物化列是状态本身;
     道具的 `item_type` 终身活(世界每帧 join 物理参数),sprite 是不变键的纯函数,
     池列 = 冗余可推导状态(两处真相 bug 面 + Init/copy_into/D10/存档 bump 全套仪式);
  2. **性能算术**——渲染 join:表 ~28B×5 行≈2 cache line 且物理相位已焐热,顶格 512 道具
     ~1µs(帧预算 0.006%);池列方案 World +1KB 反而进校验和(P6 每帧)/快照/存档三条
     必走路径,**headless RL 消费者一帧不渲染也要付账**。净账为负;
  3. **弹的非对称有实证**——`OP_SET_SPRITE`(xform.rs,transform.rs 落点)中途改弹脸
     不改判定:弹 sprite 是逐实体可变状态,只能进池;道具无任何 op 改型。
- **统一律**(裁决沿用):池里只存"世界每帧要用的状态(弹 radius)"或"逐实体独立可变的
  状态(弹/敌 sprite)";凡能从活着的不可变键推导的,留表(道具 sprite)。

**兼容性**:表格式 version bump + `content_hash` 变 → 身份三元组变 → **旧存档/回放在
头校验被拒**,预期行为非事故。世界演化零变化,但 `tables_hash`(入校验和的 World 字段)
随 `content_hash` 变 → **金向量全帧均匀漂移**;行为不变性由 §0 判别式测试证明
(勘误 2026-07-24:本节初稿误写"逐字节不变",brainstorm 对话同误,以本勘误为准)。

### 2.2 A2:bench 基线续表

按 follow-ups A2:World 实测 1.03MB / 校验和占比等数字过期,M2 帧预算决策前重跑
`cargo run --release -p stg-harness -- bench` 续表进 `docs/bench-baseline.md`。
桥刀落地后再补一行"桥编码器每帧成本"(§9 冒烟顺带测),让 §2.1 的性能账有实测背书。

### 2.3 正典 boot:`World::new_game` 下沉 core

```rust
// stg_core::step(组装层;签名 plan 期定稿,语义如下)
pub fn new_game(seed: u64, rank: i32, image: &EclImage)
    -> Result<Box<World>, TaskStartError>;
// = World::new(seed)
// + body.set_var(GVAR_RANK, rank)     // GVAR_RANK=0 已是 consts.rs ①结构常量
// + start_main(image)                  // Stage 属主,现成 API
```

- **场景实体摆放全进脚本**:`spawn_enemy`/`boss_set`/`spell_begin` 均为 `.ecl` builtin,
  boot 不需要知道任何场景——全数据驱动。
- **编译不下沉**:stg-ecl-compiler 产 `EclImage` 留消费者侧,core 只吃成品镜像
  (依赖方向不可反转)。
- rainbow 金向量的手摆 boss boot 是**冻结遗产**,一字节不动(golden 不迁移到 new_game)。
- 单测:new_game 后 frame==0、GVAR_RANK 可读回、main 任务在跑;TaskStartError 透传。

## 3. 桥刀:crate 结构(方案 A"薄壳厚核",已拍板)

```
crates/stg-godot/                cdylib → libstg_godot.so
  Cargo.toml     crate-type=["cdylib"];deps: stg-core, stg-ecl-compiler, godot(钉支持 4.6 的精确版)
  src/lib.rs     gdext 入口(ExtensionLibrary)
  src/boot.rs    纯 Rust:源码文本 → 编译 EclImage → World::new_game;错误带行列
  src/frame.rs   纯 Rust:四渲染层(弹/自机弹/敌/道具)alive 位扫 → f32 实例缓冲编码器
                 (定点→浮点唯一转换点;fields/自机走低频读口不进 MultiMesh)
  src/save.rs    纯 Rust:L1 存读字节管道 + LoadError → 可读错误映射
  src/bridge.rs  gdext 壳:WorldBridge(GodotClass, extends Node),纯胶水零逻辑
  smoke/         最小 project.godot + stg_godot.gdextension + 冒烟场景脚本 + run-smoke.sh
```

- `boot/frame/save` **零 gdext 类型**,裸 Linux `cargo test` 全覆盖(本仓 TDD 宪法);
  `bridge.rs` 只做类型转换/转发,由 headless 冒烟盖。
- 方案 B(全 gdext 单体)因不可单测否决;方案 C(双 crate 硬分层)因桥在断层线以上、
  无不变量需编译期焊死,YAGNI 否决。模块纪律沿 P1 先例(world 是模块不是 crate)。
- workspace `members = ["crates/*"]` 自动纳入;stg-core 依赖防火墙(`cargo tree`)不受影响
  (桥是下游)。

## 4. WorldBridge v1 冻结面(GDScript 可见)

| 方法 | 说明 |
|---|---|
| `new_game(ecl_source: String, seed: int, rank: int) -> bool` | 吃**源码文本**不吃路径(`res://` 的 FileAccess 归 GDScript);编译失败 → false + 行列错误日志 |
| `step_frame(buttons: int)` | 推一逻辑帧;`BTN_*` 掩码显式参数(replay/rollback 正门);常量随类导出 |
| `register_layer(kind: int, multimesh_rid: RID) -> bool` | 一次性交 RID(kind:`LAYER_BULLETS/SHOTS/ENEMIES/ITEMS` 四层);此后每帧 Rust 直推 `RenderingServer.multimesh_set_buffer`,GDScript 帧内零渲染代码 |
| `take_requests() -> Array[Dictionary]` | 本帧通道 B 请求 `{id, seq, frame, args:[i32;6]}`;分发表住 GDScript |
| `save_state() -> PackedByteArray` / `load_state(bytes) -> bool` | L1 直通;失败不动世界(先验后写);身份三元组不符 → false |
| `frame() -> int` / `checksum() -> int` | 调试/对拍读口 |
| `hud_player() / hud_boss(i) / hud_spell(i) -> Dictionary` | 低频 HUD 读口(命/雷/分/power/graze;boss_ui;符卡槽);字段实名 plan 期对代码钉死(注意 `life_state` 实名坑,见适配笔记杂项) |
| `player_pos() -> Vector2` | 自机等低频对象按 §6.4 用正常 Godot 节点当"显示器" |
| `fields_info() -> Array[Dictionary]` | 作用区低频读口(炸弹圈等演出参考),v1 不做 MultiMesh 层 |

**实例缓冲布局**:2D transform(8 float/实例;旋转 = BAM→弧度桥侧算)+ `CUSTOM_DATA`
4 float(`[0]`=sprite 索引,`[1..3]` 保留置 0);shader 按 custom_data 取图集区域。
**活槽压实**(alive 压到缓冲前段)+ `visible_instance_count` 每帧设——少画死槽。
精确 float 排列按 Godot 4.6 文档 plan 期钉死。

**道具 sprite join**:编码器循环前把 5 行 sprite 提进栈上小数组,循环内免查表(§2.1)。

## 5. 一帧的生命周期(数据流)

```
Godot _physics_process(60Hz;project 钉 physics_ticks_per_second=60)
 1. GDScript:InputMap → BTN_* 掩码
 2. bridge.step_frame(mask)
     ├ core step_with_director
     ├ frame.rs 编码四渲染层 → f32 缓冲
     ├ RenderingServer.multimesh_set_buffer(每注册层一次上传)
     └ 缓存本帧 reqs
 3. GDScript:for req in take_requests(): 分发表[req.id].call(req)
 4. GDScript:按需读 hud_* 刷 UI(低频)
```

- 表现侧动画状态(爆圈淡出/演出计时)全在 GDScript(C2 实证);表现 RNG 用 Godot 自带,
  永不碰模拟 RNG(I3)。
- 节拍器 v1 托管 `_physics_process`;`step_frame` 原语保证将来换策略不动 `.so` 面。

## 6. 错误处理(P4 精神移植到 FFI 边界)

**铁律:任何错误不得 panic 穿 FFI。**

| 类 | 处置 |
|---|---|
| 内容错误(`.ecl` 编译失败) | `new_game` 返 false + `godot_error!` 带行列的编译器原话 |
| 调用方违约(未 new_game 就 step、坏 RID、坏 kind) | no-op + false/空值 + **去重日志**(同类首次报,不逐帧刷屏) |
| 载入失败(版本/身份三元组/长度不符) | false + LoadError 可读信息;世界原状不动 |

debug 构建 core 帧内断言照常(引擎 bug 就地炸,冒烟抓);release 按 P4-c 不检查。

## 7. 时间系定位声明(时间跳跃/回溯/预测未来——超出单 world 的分层)

**World 是"一条时间线的一个切面";时间线本身是消费者的数据结构。**

```
stg-core       World = 状态 + 确定性 step + SaveBytes【不知道历史/分支的存在,I1~I7 不动】
桥后续刀        history 模块(纯 Rust):快照环(Box<World> memcpy,I7 白送)+ 输入日志
                 rewind_to(frame) = 恢复最近快照≤frame + 重演到 frame + 截断日志
                 rollout(n, 假想输入) = 抓副本推 n 帧读结果(预测未来,不落地)
Godot 场景层    技能演出(残影/滤镜),读通道 A 画
```

- **回放只留最终有效时间线**(拍板):回溯 = 日志手术(截断+续写),被放弃的时间段
  从不进回放文件 → **L2 线性回放格式零改动**,回放器不需要懂"回溯"概念;
  `World.frame` 随快照回卷,有效时间线帧号/校验和流天然连续。
- 时间跳跃(向前):落地式 = 一显示帧内连 step N 次(µs 级);预览式 = rollout 不落地。
- PRNG 随快照回卷(I3)→ 回溯后同操作同结果("诚实"回溯);要"换运气"是游戏层决定。
- **history 环 = M3 rollback 同一块机械**——游戏回溯插件顺手打 M3 地基,一份代码两处收成。
- 对 v1 的含义:时间系全是**加法 API**,不改已冻结方法;加它们重编一次 `.so`,
  不破 Godot 侧已写代码。

## 8. 测试与 DoD

**纯 Rust 层(cargo test,无 Godot)**:
- `frame.rs` 判别式单测:互异非零判别值灌池(S1 纪律),断言 8-float 排列/custom_data
  逐字段/死槽被滤/压实顺序 = 池索引升序/BAM→弧度对表;
- `boot.rs`:编译成功/失败行列透传;new_game 后 frame==0、main 在跑;
- `save.rs`:桥面往返字节恒等;坏头被拒且世界校验和不变。

**headless 冒烟(本地脚本)**:`smoke/run-smoke.sh` 以 `GODOT_BIN`(缺省 playground 4.6.3)
跑 `--headless`:加载 `.so` → `new_game(内嵌 godot_smoke.ecl, seed)` → 步 120 帧 →
断言 frame==120、有活弹、take_requests 有货、save→load→再步校验和续接 → `SMOKE OK` 退 0。
`godot_smoke.ecl` 为**脚本自举**场景(spawn_enemy 摆 boss + spell_begin + 发环),
实证全数据驱动 boot 端到端。

**DoD**:上两层全绿 + fmt/clippy 净 + **金向量逐字节不变** + CI 三平台绿
(桥 crate 入 workspace build/test;headless 冒烟 CI 接入列 follow-up)。

## 9. `.so` 产物策略("编完放那")

- `cargo build -p stg-godot --release` → `target/release/libstg_godot.so`;游戏工程约定位
  `addons/stg/bin/` + 拷贝脚本;`.gdextension` 按平台/构建型分路,
  `compatibility_minimum = 4.6`,**`reloadable = false`**(不玩热重载,语义最稳);
- Godot 侧开发零 cargo;改 Rust 才重编+重启 Godot;
- Windows 出货走既有 xwin 交叉编译出 `stg_godot.dll`(同一 `.gdextension` windows 条目),
  本刀不做,路径已通(TheOubliette 先例);
- gdext 钉精确版本进 `Cargo.lock`(支持 4.6 的版本 plan 期查实)。

## 10. 后续"Godot 刀"注意事项(提前记账,防丢)

- 节拍器:显示帧率≠逻辑帧率时,补帧上限(≤3)+超限重锚弃补(C2 查看器实证策略);
  Godot 侧还有 `max_physics_steps_per_frame` 可配,权衡 plan 期做;
- 大型演出(符卡宣言)将来上 rollback 时按 id 标 confirmed_only(§6.3),v1 无 rollback 不需要;
- `REQ_SPELL_DECLARE/RESULT/ENEMY_DEATH` 三个引擎保留 id 的 args 约定表在 `reqs.rs`
  模块文档,分发器照抄;
- 道具在版面顶端画箭头指示等是表现层状态(位置可从通道 A 推),不进 core。

## 11. plan 期待钉死清单(spec 遗留的具体值)

1. gdext(`godot` crate)支持 Godot 4.6 的精确版本号;
2. MultiMesh 2D 实例缓冲的精确 float 排列(transform 行序/custom_data 偏移,查 4.6 文档);
3. `new_game` 签名定稿(Result 错误型)与所在模块(step.rs 组装层);
4. `hud_player` 字段实名对 `PlayerState` 代码钉死(`life_state` 坑);
5. `godot_smoke.ecl` 用到的 `spawn_enemy` 参数表(对 builtins.rs 签名);
6. smoke 的 `.gdextension` 相对路径与 `entry_symbol`;
7. `ItemTypeCfg.sprite` 五行的具体贴图索引值(与将来图集约定一致即可,先占位序号);
8. `checksum() -> int` 的 u64→Godot i64 映射口径(位重解释 as-is,还是十六进制字符串读口)。
