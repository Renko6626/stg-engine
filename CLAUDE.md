# CLAUDE.md —— stg_engine 工作宪法

> 每个 session 开工前先读本文件。它是硬规则的单一入口；细节以两份权威设计文档为准。

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
- **I4 顺序**：一切遍历按池索引升序；确定性 free-list 分配；禁 `HashMap` 等无序容器参与模拟。
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
  `include_bytes!` 嵌入 → CI 再生成断言**逐位相同**；取整 round-half-to-even；表哈希进握手/回放头。
- **校验和**：**vendored FNV-1a 64**（`stg_core::checksum`，算法字节冻结，绝不走外部依赖）。
  **字段级、哈希全槽（不用 alive 掩码）、小端字节序**；`#[derive(Checksum)]`（stg-derive）从字段
  自动生成防漏。节奏：CI/金向量**逐帧**，联机随包每 **K=20** 帧采样。
- **复用槽写满硬规则**：分配/复用池槽必须写满所有字段（`Init` 结构体升级为编译期事实），
  debug 断言复用槽无遗留值——支撑"哈希全槽不掩码"。

## 仓库结构

```
Cargo.toml                       workspace（resolver=3, edition=2024）
rust-toolchain.toml              钉死 1.92.0 + rustfmt/clippy（可复现）
Cargo.lock                       【提交】—— 确定性须锁依赖版本
CLAUDE.md  README.md
design_doc.md  stg-world-design.md   权威设计（勿轻改，改动过评审）
docs/superpowers/{specs,plans}/  brainstorm 产出的设计与实施计划
docs/fixed-point-corners.md      定点数（Fx/Angle）坑与规范速查
.github/workflows/ci.yml         三平台矩阵 + 校验和对拍 + fmt/clippy + 依赖防火墙
crates/
  stg-core/         确定性内核（现仅 checksum；math/pool/world/step 随 M0）
    src/math/tables/  烘焙表原始字节（M0 生成并 commit）
  stg-derive/       proc-macro：#[derive(Checksum)]（M0）
  stg-ecl-compiler/ 离线 ECL 编译器，产出 EclImage（M1）
  stg-harness/      CLI：金向量对拍 + 烘焙表 bake/verify（允许浮点）
```

## 常用命令

```bash
cargo build --workspace                       # debug：溢出 panic、帧内断言生效
cargo test  --workspace                       # 单测 + 金向量回归
cargo fmt --all                               # 格式化（CI 用 -- --check）
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out c.txt   # 跑金向量输出逐帧校验和
cargo run -p stg-harness -- verify-tables        # 断言烘焙表字节 == commit
```

## Milestone 地图（design_doc.md §11 / §1.3）

- **M0** `stg-core` 数学核 + 池（`define_pool!`）+ step 骨架 + 快照/校验和 + `stg-derive` Checksum；
  `stg-harness` 金向量逐帧对拍。**← Phase 1 当前起点**
- **M1** `stg-core` ECL VM + syscall 表；`stg-ecl-compiler` Rust DSL 拼字节码，跑通一张非平凡符卡。
- **M2** `stg-godot`（gdext）WorldBridge + MultiMesh + 请求分发器 —— **phase 后续，暂不建 crate**。
- **M3** 环形快照 + 本地回滚 harness（延迟/输入扰动/校验和风暴）。
- **M4** `stg-net`（UDP + 会话/重同步）—— **phase 2 起点**。
- **M5** `stg-py`（PyO3 headless 并行 env）。

> 本仓当前**只建 Phase 1 三 crate + stg-derive**；godot/py/net 到各自 milestone 再加。

## 开发工作流（全流程）

- **每个 milestone 走 superpowers 环**：需要设计先 brainstorming → writing-plans → executing-plans
  （TDD + 复审检查点）→ requesting-code-review → finishing-a-development-branch。
- **TDD 强制**：确定性项目里测试即规格；金向量本身就是 §9 的回归向量，回归集只增不减。
- **分支**：trunk-based，每个 milestone/任务开短命特性分支，合入前跑绿 CI。
- git commit 结尾附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

## 改动前自检清单（贴身版）

1. 我改的代码在断层线以下吗？若是 stg-core：没引入浮点/时钟/宿主 RNG/无序容器吗？
2. 新增了 World 字段吗？→ 它自动进校验和了吗（derive）？复用槽写满了吗？容量进 D10 预算了吗？
3. 改了 step 顺序 / 碰撞矩阵 / op 清单 / 烘焙表 / 校验和算法吗？→ **过评审 + 可能 bump engine_ver**。
4. 有对应的测试 / 金向量回归吗？CI 三平台会绿吗？
