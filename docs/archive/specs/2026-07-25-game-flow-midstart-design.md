# 整局流程刀——多文件编译 + mark 中段启动 + Loadout + 表现锚点

> **状态**:brainstorm 拍板(2026-07-25)。动机链:用户问"跨关卡状态保留怎么做"→ 结论是
> 方案 A 下该问题消失 → 真正的靶子是 **practice/中段启动** + 整局脚本的书写形态。
> 本刀 = 引擎侧地基;背景 STD 式 mini-VM **明确下一阶段**(§7)。

## 0. 顶层裁决:方案 A——整局一 World 一镜像

- **跨关卡状态保留不是引擎问题**:`power/lives/bombs/score/graze` 全在 `PlayerState`
  (World 内,进校验和/快照/存档)。状态丢失的唯一途径是重启 World——**所以不重启**。
  "关卡"下沉为脚本层概念:`main` 顺序调 `stage1(); stage2(); …`,World 全程活着。
- **一局 = 一个 EclImage = 一个 content_hash**(身份三元组承重墙,存档头/握手/回放头
  单镜像 hash 不动)。
- **否决记录**:B(每关一 World + Carry 结构,ZUN 式)——回放被迫分段、rollback 不能
  跨关底、new_game 长 carry 变体,整套契约长"段"概念;C(一 World 换镜像)——身份碎
  成"镜像序列+切换帧清单",复杂度只是搬家。均否。
- **回放/握手身份 = `(seed, rank, start, loadout, image_hash)`**,纯定宽整数。

## 1. 多文件编译(化解"巨型 ecl"痛点)

"一局一镜像"说的是**产物**,不是源文件。每面一个 `.ecl`(恰好是 ZUN 的源码布局),
编译期合并:

- 编译器新入口 **`compile_units(&[(文件名, 源文本)])`**:各文件独立 lex/parse
  (**报错带各自文件名+行列**,这是不能傻拼接文本的原因),AST 合并后 typeck/codegen
  走原路。编译器保持纯函数无 I/O(断层线同款纪律),文件收集归调用方。
- 既有事实支撑:codegen 先声明全部 sub 再生成 body,`build()` 按名排序出 canonical
  SubId,**产物不依赖声明顺序**(codegen.rs 模块文档)→ AST 级合并后端零改;收集顺序
  大概率不影响镜像 hash,plan 期以测试钉死(两种顺序 → 同 image 字节)。
- 新诊断:跨文件撞名 sub → 重复定义报错,指出两处位置(const/xformdef 同规则,plan 期核)。
- **收集约定(拍板)**:harness `check`/桥 `boot` 接路径——**是目录就收全部 `*.ecl`
  按文件名字节序排序**,是单文件照旧(金向量/现有场景零改)。零配置格式,无清单文件。
- 明确不做:`include` 语法(逼编译器做路径解析,破纯函数)、模块系统/命名空间(扁平
  全局名字空间 + 撞名报错够用)。

## 2. mark 中段启动("从游戏中间某处开始")

**语义拍板:中段启动是"规范态"开局(符卡练习语义),不是"仿佛打过来的状态"。**

### 2.1 语言:`mark` 语句

```
sub main() {
    stage1();
    mark(MARK_S2);
    stage2();
    mark(MARK_S4_MID) {            // 可选补偿块:跳入时执行,正常流程跨过
        set_global(GVAR_ROUTE, 1); // 被跳过的全局变量在此补
    }
    s4_midboss(); …
}
```

- 编译三规则(违者编译错):① `mark` 只允许出现在 `sub main` 体的**顶层语句位**
  (不进 if/while/for 块——跳进嵌套调用要重建调用栈,噩梦;main 顶层 = 深度 0,
  ip 一搁即合法);② id 是编译期常量、**全镜像唯一、非 0**(0 = 从头);③ main 顶层
  `var` 声明**不得先于任何 `mark`**(任务帧零初始化,跳入后 mark 前初始化的局部全是
  0——编译期封死这个坑)。
- 降低 = **landing pad**:`jmp after; landing: <注入补偿><作者块>; after:`——正常流
  一步跨过垫片零感知,引擎跳入恰落垫上。
- 文档注明:mark 之前正常流程写的全局变量跳入时为 0,需要的在 mark 块补。

### 2.2 镜像与开机

- EclImage 长**标记表** `mark_id → ip(main 内落点)`。镜像编码格式细节(版本/空表
  编码)plan 期钉;金向量输出是 World 校验和流,不含镜像 hash,不受影响。
- **`new_game_at(seed, rank, start, loadout, image)`**:start≠0 → 查标记表把根任务
  ip 搁到落点,查无 → **开机期报错**(`TaskStartError` 新变体,不是运行期 Fault);
  现有 `new_game(seed, rank, image)` 改为委托 `new_game_at(…, start=0,
  Loadout::default())`,现有调用方零改。
- 早期方案 `GVAR_START` + 脚本 if 链分派**否决**(用户裁:不优雅;每个起点要抄后续
  流程/集中式分派),引擎跳 ip 取代。

## 3. Loadout(装备上行到开局参数)

装备是**玩家在菜单调好的数据**,不是脚本内容,从建世界的门进:

```rust
pub struct Loadout { pub character: u8, pub power: u16, pub lives: u8, pub bombs: u8 }
// score/graze 恒 0 开局,不进结构。
// Loadout::default() = (0, 0, 3, 3)——把 PlayerState::spawn 的硬编码收编成单一来源。
```

- 钳位:power/lives/bombs 越界 → 钳(P4-b);**character 越 `characters` 表界 →
  开机期报错**(宿主时间,响亮失败优于静默钳到别的机体)。
- 桥 `boot` 面相应长参数(WorldBridge 冻结面变更,C17 记账口径,plan 期钉具体形态)。
- **连锁裁决:账面 builtin 四件套缩编为一件 `add_score`**(关底 bonus/结算记账必须在
  世界内发生)。set_power/set_lives/set_bombs 的唯一场景(practice 装备)已上行,撤下
  记 follow-up(将来"道中事件奖命"类脚本需求出现再开)。add_score 钳位语义 plan 期钉
  (负值/饱和)。

## 4. 表现锚点四字段 + 声明式 builtin 三只(本刀核心交付之一)

**动机**:req 是一次性输出缓冲,存读档/回滚/中段启动后表现层失联(读档进 4 面音乐是
哑的)。锚点 = 住 World 的表现状态,"跳进来的世界要长得像一路打过来的世界"。

```
World 字段:  bgm_id: u16    bg_id: u16    bg_phase: u16    bg_phase_frame: u32
```

- 全进校验和(P6 全量;先例 `facing`)/快照/存档;`copy_into`/尺寸哨兵/D10 走自检
  清单常规流程;save 载荷变 → `SAVE_FILE_VER` bump(无存量用户,零成本窗口)。
- **声明式 builtin**:`bgm(id)` / `bg(id)` / `bg_phase(n)`,void 裸语句,降低语义 =
  **写字段 + 发 req**(桥既能每帧读字段也能收边沿通知);`bg_phase(n)` 同时自动把
  `bg_phase_frame` 盖为当前帧。req 种别号(REQ_BGM/REQ_BG/REQ_BG_PHASE)收进
  consts.rs C14 注册表成为引擎常量(编译器因此认识它们,§5 的前提)。
- 桥侧开四读口(HUD 读口 B18 同族)。
- **校验和影响如实承认**:新增受检字段 → 金向量逐帧值**整体平移**(A1 表 hash 同款
  性质)。行为不变靠判别式测试证明(plan 期钉形态),本地 before/after 对拍留档,
  三平台互拍闸照常。

## 5. mark 自动补偿(用户提案:没手写就编译器扫最近的)

- 编译器把 main 的**同步调用链线性展开**(main 顶层按序递归进被同步 call 的 sub,
  visited 防环),收集沿途 `bgm()`/`bg()`/`bg_phase()` 声明位置。
- 每个 `mark` 取其**之前最近**的声明各一条注入垫片(注入在作者块**之前**);作者块里
  手写了对应声明的不注入,三类各自独立判断。
- 细则:`bg_phase` 声明若早于最近一条 `bg` 声明(属于旧背景的 phase)则不注入 phase,
  具体 plan 期钉。注入的 `bg_phase` 盖跳入帧的帧戳——语义恰好正确(本段从跳入帧起算)。
- **诚实边界(文档明写)**:自动扫描只认**顶层线性位**的声明——if/难度分支里的、
  `spawn`/async 任务里的(如 boss 登场任务切 boss 曲)不参与;escape hatch = mark 块
  手写覆盖。这是"推荐默认 + 手动兜底",不是全自动保证。

## 6. 转场/结算页协议(纯约定,引擎零改)

**原则:世界时间线只装 gameplay;结算页/菜单住墙钟时间。**World 是纯被动状态机
(I7:无线程无定时器),宿主不调 `step` 就是完美冻结。

```
脚本关底:  add_score(bonus);              ← 改账必须在世界内(进校验和/回放)
           emit_req(REQ_STAGE_CLEAR, …);   ← 挂牌
           stage2();                        ← 直接续行,对暂停零感知
桥:        step 后 take_requests() 见牌 → 停手不再 step → Godot 原生 UI 画结算页
           (数据走读口)→ 玩家确认 → 恢复 step → 世界里 stage2 第一帧才发生
```

- 回放 = 逐帧输入,暂停贡献零帧 → 回放文件里没有结算页,重播直接穿过(恰是东方回放
  实际行为)。账和展示分层:bonus 在世界内一次记清,页面上的滚分动画是表现层的戏。
- spec 附:**req 种别号注册段**——REQ_STAGE_CLEAR/REQ_BGM/REQ_BG/REQ_BG_PHASE 的号段
  分配规矩(plan 期定初值),防散养撞号。
- 联机暂停同步是 M4 既有议题,本协议不新增问题。

## 7. 背景 STD 式 mini-VM:明确下一阶段(本刀不做)

用户查证提案 + truth 文档确认:ZUN 的 STD = 时刻调度指令流,`goto label @ time`
**同时设 ip 和时钟**,循环 = 回跳+回拨钟;机器状态仅 `(ip, clock)`,纯 STD 控制流
静态 → 任意帧状态可解析求出(段内插值 + 循环取模)。**机器模型采纳 STD 式**,但:

- 它是**表现层资产**:住 stg-godot/Godot 侧,可用浮点,不进校验和不占任务槽——
  背景永不反馈进玩法,断层线判据。
- **嫁接 phase 分段保寻位**:外部触发破环(boss 死才继续)会让状态依赖信号历史,而
  读档后历史丢失。约定:背景脚本按 phase 分段,段内 wait/loop/jump 随便,跨段转移
  只由 `bg_phase` 驱动。寻位配方:`local_t = 世界帧 - bg_phase_frame`,从该段入口
  解析执行到 local_t。变长 boss 段 = 段尾无限 loop,phase 切换破环。段是无记忆的;
  需要更多历史就拆更多 phase。
- **本刀只交付 §4 锚点契约**;解释器+文本格式归 Godot 场景刀或独立背景刀。契约今天
  钉死,将来 mini-VM 再花哨也不回头改世界。

## 8. 清弹 builtin:不做,记 follow-up

practice 不需要;整局关底转场才需要,且语义有内容层设计空间(直接消/转点/护盾帧),
等真实整局脚本落地再定。

## 9. 测试与 DoD

- **compile_units**:跨文件调用编译通过并可跑;撞名 sub 双位置报错;收集顺序不变性
  (同一组文件两种顺序 → 镜像逐字节相同);目录收集(排序)与单文件路径各一测。
- **mark**:编译三规则各一条报错测试;判别式——`start=M` 开局跑若干帧,世界状态与
  "手工从垫片语义推出的预期"相符(垫片执行、补偿注入、后续流程启动、跨过的不启动);
  正常流程(start=0)跨过垫片零执行。
- **Loadout**:钳位各字段;character 越界开机错;`new_game` 委托默认值路径下自机
  状态与改前逐位一致。
- **锚点**:三 builtin 写字段+发 req+帧戳各一测;判别式证明行为不变(锚字段不被任何
  相位读、仅 builtin 写——形态 plan 期钉);金向量整体平移如实记录(§4)。
- **金向量**:镜像无 mark、默认 Loadout → 世界演化行为不变;校验和流因新字段平移,
  本地 before/after 行为判别 + 留档,三平台互拍照常。
- **gen-ecl-meta 重跑**:新 builtin(bgm/bg/bg_phase/add_score)进 ecl-meta.json +
  ecl-lang.md 生成段,编辑刀漂移闸自动押运;坑清单补 mark 三规则与补偿边界。
- fmt/clippy/尺寸哨兵/copy_into/防火墙全绿;PROGRESS.md 收口。

## 10. plan 期钉死清单

1. `compile_units` 签名与 CompileError 的文件名字段现状(lex/parse 错误结构);
   const/xformdef 跨文件撞名规则核实;
2. EclImage 标记表编码(格式版本/空表编码/content_hash 影响面)+ `TaskStartError`
   新变体名;
3. `mark` 词法形态(关键字 vs wait_spell 式前瞻)+ landing pad 具体降低指令序列;
4. Loadout 在 `PlayerState::spawn` 的接线(spawn 签名动不动)+ 桥 boot 面形态;
5. 四锚点字段落位(WorldBody 哪一段)+ req 载荷布局 + REQ_* 号段初值;
6. mark 补偿扫描的实现落点(codegen 前的独立趟?)+ bg/bg_phase 新旧背景细则;
7. add_score 钳位语义(负值/饱和);
8. 行为不变判别式测试形态(锚字段"仅 builtin 写"的守法测试怎么写)。
