# CLAUDE.md —— stg_engine 工作宪法

> 每个 session 开工前先读本文件。它是硬规则的单一入口；细节以两份权威设计文档为准；
> **当前进度/下一步以根目录 [`PROGRESS.md`](PROGRESS.md) 为唯一权威**（本文只写不变的东西）。

## 这是什么

**Godot 前端 + Rust 确定性内核的 2D 弹幕 STG 引擎。** 一等目标：rollback 联机、逐帧回放、
headless 高速模拟。核心性质是**确定性**——同一份 `(初始状态, 静态脚本, 输入序列)` 在所有受支持
平台产生 **bit 级完全一致**的世界演化。

**权威设计文档（冲突时以后者为准）**：
- `design_doc.md`（v0.3）—— 总纲：架构、断层线、确定性契约、ECL 层、网络、测试。
- `stg-world-design.md`（v1.0）—— 世界层实施蓝图，经 20 轮评审拍板；与总纲冲突处以本文为准。

## 阶段优先级与 Phase 1 DoD

目标排序：**确定性（回放/CI 基底）→ headless/单机手感 → rollback 联机（北极星，非首交付）**。

- **Phase 1 交付** = `stg-core` + `stg-ecl-compiler` + `stg-harness`，本地把金向量跑通。
- **Phase 1 的 Definition of Done**：一条复刻真实东方符卡的病态诊断场景（**金向量**）在
  **x86_64 与 aarch64 上逐帧校验和完全一致**。不是"引擎能跑"，是"跨平台逐帧对拍一致"。
- **交付目标分阶段、不变量（I1–I7）不分阶段**：从第一天起完整持有全部不变量，rollback 后续
  接入才近乎免费。

## 断层线：靠 Cargo 依赖图编译期焊死，不靠自律

```
  stg-godot(gdext) ─┐                stg-ecl-compiler ──产出 EclImage──┐
  stg-py(PyO3)     ─┼── 依赖 ─► stg-core ◄────────────────────────────┘
  stg-harness(CLI) ─┘          (确定性内核，本仓 crates/stg-core)
                     stg-core 的 Cargo.toml 里【没有】godot/std-time/rand/libm
```

- **`stg-core`**：断层线以下全部。不 import Godot、不读系统时钟、不用浮点、不碰宿主 RNG——
  这些依赖**在 Cargo.toml 层面就不存在**。CI 有一步 `cargo tree -p stg-core` 断言这条防火墙。
- **浮点只活在断层线以上**：烘焙表用 f64 生成，但生成器住 `stg-harness`（`bake-tables`），
  产出的**原始字节** commit 进 `stg-core/src/math/tables/`，core 只 `include_bytes!` 消费。
- **依赖方向单向且世界无知**：`ECL → world`；world 不 import 任何 ECL 类型，不知道"任务"存在
  （P1）。`World.tasks`（ECL 类型）物理上住组装层 `World` 里以满足 memcpy 快照。

## 七条不变量 I1–I7（任何提交不得违反）

- **I1 数值**：唯一标量是定点数 **Q16.16（i32）**；`f32/f64` 不得出现在断层线以下。
- **I2 角度**：**BAM u16**（一圈 65536），三角函数一律查表。
- **I3 随机**：PRNG（PCG32）状态是 `World` 字段，随快照回滚；表现层用另一颗独立 RNG。
- **I4 顺序**：一切遍历按池索引升序；确定性分配（存活掩码最低空位、无 free-list）；禁 `HashMap` 等无序容器参与模拟。
- **I5 协程**：模拟协程完整状态（ip+栈+局部）位于**可 memcpy 的扁平内存**；禁用 async/await 承载模拟。
- **I6 时间**：逻辑固定 60 Hz，一切计时用整数帧，`step` 不接受 delta。
- **I7 布局**：`World` 内无指针/引用/堆容器；实体互引用 generation-index 句柄；**快照 = 整块字节复制**。

## 六条原则 P1–P6（世界层，改原则需评审）

- **P1** 边界：world 是 `stg_core::world` 模块（不拆 crate），但接口按 crate 级纪律书写——
  调用方**永不直接触碰池内存**，只走安全读/写 API。
- **P2** step 所有权：§3.5 固定顺序写在**组装层** `stg_core::step`；world 只出 `pub(crate)` 相位函数；
  debug 有 **PhaseGuard** 押运时序；step 3 是**导演槽**（默认租户 = ECL 任务运行器）。
- **P3** 单线程：单 world 内一切演化永远单线程；并行只在 world **之间**（RL env / 多回放）。
- **P4** 错误三铁律：(a) 资源耗尽→确定性降级不 panic（返回 NULL+错误码，计数参与校验和）；
  (b) 调用方违约→确定性安全结果（悬垂句柄视同已失效，坏参数 no-op+计数）；
  (c) 引擎自身 bug→**debug 帧内断言就地 panic**，release 不检查（靠 CI debug 金向量挡在合入前）。
- **P5** 无回调：world 不持有任何回调/函数指针/外部注册表（违反 I7）。定制行为只能是"数据"或
  "上层在相位间读事件后自行动作"。母文档 `on_died` 回调构想已否决。
- **P6** 全量校验：住在 World 里的字段就参与校验和，无例外（含 `facing` 等"纯表现"字段）。
  唯一例外是三条纯输出缓冲（`reqs`/`hits`/`frame_events`）+ debug 的 `phase_guard`，每个 skip
  必须在 derive 属性里给理由字符串。
  > **三条缓冲的名字设计与代码一致**（`frame_events`/`hits`/`reqs`）——`frame_events` 曾在
  > 代码里叫 `events`，拿设计去 grep 搜不到，2026-09-03 的技术债刀已把字段改回同名（D2 销）。

## 确定性契约要点（细节见 D1/D11/§2.1）

- **定点数学核**：`Fx = Q16.16(i32)`、`Angle = BAM(u16)`，newtype 强类型（`repr(transparent)`）。
  `mul = ((a as i64 * b as i64) >> 16) as i32`（算术右移）。sin/cos 查 16384×i32 四分之一波表；
  `atan2` 整数 CORDIC **钉死 16 轮**；`isqrt` 整数牛顿法。碰撞用**平方距离**比较，不开根。
- **定点乘法规范（重要，涉及后续一切运算）**：`Q16.16 × Q16.16 = Q32.32`（小数位相加 16+16=32）。
  `Fx::mul` = 先得 Q32.32（i64 中间量）再 **`>>16` 归一化回 Q16.16(i32)**——这一步既丢低 16 位小数、
  又是溢出的来源（结果须 ≤ ±32768，故**仅当至少一个操作数 ≤ ~1.0** 时安全：`speed × 单位向量`、
  `位移 × easing-t`、`坐标 × 归一化权重`）。**平方距离 / 模平方 / 点积等"离开屏幕空间"的量**：
  保留 Q32.32 于 **i64、不归一化**（`math::geom::len_sq`），既不溢出又不丢精度；比较时 `r²`(`r.raw()²`)
  同为 Q32.32，直接比、不开根。通用律：`Q(m).f × Q(m).f = Q(2m).(2f)`，`>>f` 才回 `Q(m).f`；
  **保留双宽 = 累加器模式**。两个大坐标相乘却 `>>16 as i32` 塞回 `Fx` = 必溢出的经典翻车。
  **完整坑表（加减语义 / Angle 回绕 / ECL 脚本 / 大数参数方程策略）见 [`docs/fixed-point-corners.md`](docs/fixed-point-corners.md)。**
- **烘焙表纪律**：**绝不在各平台构建期用浮点现生成表**。生成一次 → commit 原始字节 →
  `include_bytes!` 嵌入 → CI 再生成断言**逐位相同**；取整 round-half-to-even；表哈希进握手/回放头
  （身份三元组 = `ENGINE_VER`〔lib.rs，bump 须过评审〕+ 表 `content_hash` + 镜像 `content_hash`）。
- **校验和**：**vendored FNV-1a 64**（`stg_core::checksum`，算法字节冻结，绝不走外部依赖）。
  **字段级、哈希全槽（不用 alive 掩码）、小端字节序**；`#[derive(Checksum)]`（stg-derive）从字段
  自动生成防漏。节奏：CI/金向量**逐帧**，联机随包每 **K=20** 帧采样。
- **金向量闸门的能力边界（重要，定测试策略）**：`determinism-gate` 只把三平台校验和流**互相比**
  （`ci.yml` 的 `diff -u`），**仓库无 committed 基线** → 它抓的是**跨平台分歧**，抓不了**行为回归**：
  改错行为会产出"三平台一致但都错"的流、闸门照绿。故**行为正确性只能靠单测守**，金向量守不了。
  推论：任何"招牌不变量"（如 D8 体碰/受击双半径）必须有**判别式单测**（几何取值能区分对错），
  圆心重合式测试对半径映射是瞎的——M0-7 变异检验已实证（对调 radius↔hurtbox，圆心重合测试仍绿）。
  机制详解（新字段默认入校验的保证 / 编译期 vs 运行时 / 性能）见 [`docs/checksum-mechanism.md`](docs/checksum-mechanism.md)。
- **复用槽写满硬规则**：分配/复用池槽必须写满所有字段（exhaustive `Init` 编译期强制），
  配宏全覆写单测——支撑"哈希全槽不掩码"。`define_pool!` 是 stg-derive proc-macro，存活掩码即分配器。

## 仓库结构

```
Cargo.toml                       workspace（resolver=3, edition=2024）
rust-toolchain.toml              钉死 1.94.0 + rustfmt/clippy（可复现）
Cargo.lock                       【提交】—— 确定性须锁依赖版本
CLAUDE.md  README.md
PROGRESS.md                      【进度入口】当前位置/下一步/里程碑史的唯一权威（milestone 收口必更）
design_doc.md  stg-world-design.md   权威设计（勿轻改，改动过评审）
docs/follow-ups.md               【接手先读】技术债与待办（复审判定可延后的，逐条核实过）
docs/superpowers/{specs,plans}/  brainstorm 产出的设计与实施计划（历史记录）
docs/fixed-point-corners.md      定点数（Fx/Angle）坑与规范速查
docs/checksum-mechanism.md       校验和机制 + "新字段默认入校验" 保证
docs/pool-memory-layout.md       池 SoA 布局与缓存精算（热路径驻 L2）
docs/xform-ops.md                弹变换 op 速查表（编号即契约；作者视角参考）
docs/ecl-lang.md                 【ECL 脚本作者第一入口 / agent 必读】.ecl 表层语言手册的**薄索引**
                     （全景 + 五条静默坑 + 该读哪一篇；全仓十余处链接指着它，勿改名）
docs/ecl-lang/                   手册正文，按教学顺序 8 篇：1 hello-danmaku / 2 tasks / 3 enemy
                     / 4 bullets / 5 types / 6 spell-and-stage / 7 reference / 8 errors。
                     内建函数生成段住 7-reference.md（gen-ecl-meta 写入，改 builtins.rs 后重跑
                     同步）；每篇的 ```ecl 围栏都被 cargo test -p stg-harness 真编译
docs/ecl-ops.md                  ECL 字节码层速查（op/syscall/fault 码；VM/编译器开发用）
docs/zun-ecl-v2-reference.md     ZUN ECL V2 指令/变量表本地副本 + 逐条对照（源 Priw8）
docs/bench-baseline.md           性能基线（step 曲线/快照/校验和账；大改后重跑续表）
docs/bridge-adaptation-notes.md  外接适配坑记录（每接一个消费者踩的坑；M2 WorldBridge 先读）
docs/render-contract.md          表现层契约权威（图集网格/stride 12/请求分发/锚点双表示；
                     场景刀首要读者：美术 + Godot 壳作者）
editors/vscode/stg-ecl/          VS Code 扩展：高亮/补全/签名/hover，数据源 ecl-meta.json（编辑体验刀）
.github/workflows/ci.yml         三平台矩阵 + 校验和对拍 + fmt/clippy + 依赖防火墙 + .ecl 单平台门禁
crates/
  stg-core/         确定性内核（断层线以下）
    src/math/        定点核：fx/angle/trig/cordic/easing/geom/isqrt/codec
    src/math/tables/ 烘焙表原始字节（harness 生成并 commit，core 只 include_bytes!）
    src/checksum.rs  vendored FNV-1a 64（D11）
    src/rng.rs       vendored PCG32（I3）
    src/{bullets,shots,enemy,field,items,player}.rs  实体数据模块（前五个是 define_pool! 实例；池即层）
    src/{boss,tables}.rs  boss 公告板（A2）/ WorldTables 静态数据层（shottype+道具+角色参数+appearance；
                     &'static 参数穿线不进 World，M0-15/17）
    src/xform.rs      变换段池（D4；手写特例，段即分配单位）
    src/{input,events,reqs}.rs                 输入抽象 / hits+events 缓冲 / 通道 B 请求（RenderReq）
    src/consts.rs    脚本可见引擎常量注册表（C14：①结构常量/②表符号；lib.rs 另有 ENGINE_VER）
    src/spell.rs     符卡计器机构（记账归引擎：SpellSlot 计时/衰减/破卡血线/伤害下钳/结算入分
                     /boss_ui 自动喂；模式随卡生死靠 spell_bound+epoch；控制归脚本，2026-07-24）
    src/world.rs     WorldBody 字段所有权 + 写 API + 读口(frame/frame_events/take_requests/rand_range
                     /表现锚点四读口 bgm_id·bg_id·bg_phase·bg_phase_frame) + push_* + PhaseGuard + 场界常量(pub)
    src/world/       【模块结构镜像相位骨架】player(相1+3，shottype 表驱动发弹) / transform(相4)
                     / integrate(相5) / collide(相6) / settle(相7) / cleanup(相9) / motion(D3 运动写 API)
                     / view(通道 A `WorldView` 零拷贝只读视图)
    src/ecl/         【M1】栈机 VM：task(协程池 256) / ops(op 表+ARITY) / vm(解释核+相2调度租户)
                     / image(EclImage) / syscall(号表+白名单沙箱绑定)
    src/step.rs      P2 组装层：§3.5 宪法顺序的唯一持有者 + World{body,tasks} + 快照
                     + 正典开机 new_game/new_game_at(Loadout 装备 + mark 中段启动,2026-07-25)
  stg-derive/       proc-macro：#[derive(Checksum)] + define_pool!
  stg-ecl-compiler/ ECL 编译器：src/lang/【M1.9 表层语言】lex/parse/typeck(三型)/slots(静态槽分配)
                    /codegen(含 mark 垫片降低+锚点自动补偿)/units(多文件编译单元,目录整取按名排序)
                    —— .ecl 源码启动时编译成 EclImage；lib.rs builder = codegen 后端
  stg-harness/      CLI：golden 两段金向量（scenes/rainbow.ecl 符卡）+ bench 基线 + 烘焙表 bake/verify（允许浮点）
  stg-godot/        M2 桥：WorldBridge gdext cdylib（boot/frame/save 纯模块+壳；smoke/ headless 冒烟）
godot/            真 Godot 工程（场景刀）：场景树/四层 MultiMesh 渲染链/请求分发器/HUD/
                  demo 局 .ecl（杂兵+风铃卡 boss）；渲染契约见 docs/render-contract.md；
                  冒烟 godot/smoke/run-smoke.sh
  README.md       【异机开跑第一入口】clone 后怎么build/跑/排错（Windows 与 Linux 各一条路径口径）
scripts/find-godot.sh   两个冒烟共用：选 Godot 二进制（≥4.6 版本闸）+ 超时上限
                  （低版本不加载扩展 → headless 挂死不报错，坑档 G14）
```

## 常用命令

```bash
cargo build --workspace                       # debug：溢出 panic、帧内断言生效
cargo test  --workspace                       # 单测 + 金向量回归
cargo fmt --all                               # 格式化（CI 用 -- --check）
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out c.txt   # 跑金向量输出逐帧校验和
cargo run -p stg-harness -- verify-tables        # 断言烘焙表字节 == commit
cargo run -p stg-harness -- check <f.ecl|目录>   # .ecl 只编译不跑,行列报错(目录=多文件整局)
cargo run -p stg-harness -- run <f.ecl|目录>     # 跑起来看结果:计数/峰值/末帧+--at F 单帧弹表
                                             # ⚠ task fault 会打出来并退非零码(别处一律静默)
cargo run -p stg-harness -- gen-ecl-meta         # 改 builtins.rs 后同步元数据/文档两生成 sink
cargo run -p stg-harness -- serve [--ecl f]  # WebSocket 查看器(默认风铃卡;--ecl 跑自己的脚本,
                                             # 每次连接重编 ⇒ 刷新浏览器即热重载。ssh -L 转发)
cargo run --release -p stg-harness -- storm      # 恢复重演风暴闸(存档正确性)
bash crates/stg-godot/smoke/run-smoke.sh     # 桥级冒烟(桥面回归)
bash godot/smoke/run-smoke.sh                # 真工程冒烟(demo 局两次开机:正常/中段)
cargo build -p stg-godot && godot --path godot   # 真工程开玩(异机 clone 先读 godot/README.md)
```

> 冒烟脚本的 `GODOT_BIN` 依次取:环境变量 → PATH 里的 `godot` → 本开发机绝对路径。
> **异机/Windows** 上跑请先读 [`godot/README.md`](godot/README.md)——`.gdextension` 认的是
> **cargo 原生产物布局**(`target/{debug,release}/`),Windows 上带 `--target` 三元组编译反而会
> 让 Godot 找不到 DLL。

## Milestone 地图（design_doc.md §11 / §1.3）

> 本节只写各 milestone 的**静态定义**。走到哪 / 下一步候选见 [`PROGRESS.md`](PROGRESS.md)，
> 开工前先读它 + `docs/follow-ups.md`。

- **M0 ✅**（2026-07-14~18，18 刀）`stg-core` 数学核 + 池 + step 骨架 + 快照/校验和 + Checksum derive
  + 世界层全机制（碰撞/结算/道具/变换/批量/shottype 表/WorldTables）+ 金向量对拍 + bench 基线。
- **M1 ✅**（2026-07-18）ECL 栈机 VM + 协程池 + syscall 白名单沙箱；**M1.9 ✅**（2026-07-19）
  `.ecl` 表层语言 + 编译器（三型/具名函数/静态槽分配），风铃卡狗粮进金向量二号；
  **整局流程刀续**（2026-07-25）多文件 `compile_units`/`mark` 中段启动+自动补偿/
  `Loadout`+`new_game_at`/表现锚点四字段（spec `2026-07-25-game-flow-midstart-design.md`）。
- **M2 ✅**（2026-07-24~26）`stg-godot` WorldBridge（桥刀，2026-07-24）+ 真 Godot 工程竖切
  （场景刀，2026-07-26：场景树/四层 MultiMesh 渲染链/请求分发器/HUD/demo 局 .ecl/双冒烟）
  全部落地——headless 可玩可验证一整段 demo 局（杂兵段 → 风铃卡 boss 战 → 挂牌结算）。
- **M3** 环形快照 + 本地回滚 harness（延迟/输入扰动/校验和风暴）。
- **M4** `stg-net`（UDP + 会话/重同步）—— **phase 2 起点**。
- **M5** `stg-py`（PyO3 headless 并行 env）。

> 本仓当前建 Phase 1 三 crate + stg-derive + `stg-godot` + `godot/`（M2 全落地）；py/net
> 到各自 milestone 再加。

## 开发工作流（全流程）

- **每个 milestone 走 superpowers 环**：需要设计先 brainstorming → writing-plans → executing-plans
  （TDD + 复审检查点）→ requesting-code-review → finishing-a-development-branch。
- **TDD 强制**：确定性项目里测试即规格；金向量本身就是 §9 的回归向量，回归集只增不减。
- **分支**：trunk-based，每个 milestone/任务开短命特性分支，合入前跑绿 CI。
- git commit 结尾附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

## 改动前自检清单（贴身版）

1. 我改的代码在断层线以下吗？若是 stg-core：没引入浮点/时钟/宿主 RNG/无序容器吗？
2. 新增了 World 字段吗？→ 它自动进校验和了吗（derive）？复用槽写满了吗？容量进 D10 预算了吗？`copy_into` 同步了吗（尺寸哨兵测试会红）？
3. 改了 step 顺序 / 碰撞矩阵 / op 清单 / 烘焙表 / 校验和算法吗？→ **过评审 + 可能 bump engine_ver**。
4. 有对应的测试 / 金向量回归吗？CI 三平台会绿吗？
5. 这刀是 milestone 收口 / 交班吗？→ `PROGRESS.md` 更新了吗（史加一行 + 重写「现在」段）？
