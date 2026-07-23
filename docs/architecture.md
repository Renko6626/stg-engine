# stg_engine 架构与子系统

> 一页看懂整个引擎的形状——给"构思下一步工程"用的地图。**只画骨架 + 指路**，深度细节以权威文档为准
> （见[文末索引](#权威文档索引)），不在此复述以免漂移。当前进度/下一步见 [`PROGRESS.md`](../PROGRESS.md)，
> 硬纪律见 [`CLAUDE.md`](../CLAUDE.md)。

## 断层线：五层一线

```
  ┌─────────────────────────────────────────────────────────────┐
  │  表现层 / 消费者   Godot(gdext) · headless harness · py env   │  断层线以上
  │                    （允许浮点/时钟/线程；世界无知）            │  (M2/M5)
  └───────────────▲───────────────────────────┬─────────────────┘
        通道A 状态视图 · 通道B 渲染请求队列    │  每帧一份 InputFrame
  ┌───────────────┴───────────────────────────▼─────────────────┐
  │  确定性内核 stg-core（断层线以下）                            │
  │    world（实体池 + 相位） ◄── ECL VM（栈机协程） ◄── EclImage │
  │    math 定点核 · checksum · rng · WorldTables 数据层          │
  └─────────────────────────────────────────────────────────────┘
```

- **断层线由 Cargo 依赖图编译期焊死**：`stg-core` 的 `Cargo.toml` 里**没有** godot / std-time / rand / libm。
  CI 一步 `cargo tree -p stg-core` 断言这条防火墙。浮点只活在断层线以上（烘焙表在 harness 生成、字节 commit）。
- **向下唯一入口**：每帧一份 `InputFrame`。**向上两个出口**：通道 A（状态内存视图，读）+ 通道 B（渲染请求队列）。
- **核心方程**（`step` 不接受 delta，逻辑固定 60 Hz）：

  ```
  world[n+1] = step(world[n], &WorldTables, &EclImage, input[n])
  ```
  `WorldTables` / `EclImage` 是**只读静态数据**，按帧传引用、**不进 World**（I7）；两机一致由内容哈希保证。

## 七不变量（确定性契约，任何提交不得违反）

| # | 不变量 | 一句话 |
|---|---|---|
| I1 | 数值 | 唯一标量 = 定点 **Q16.16(i32)**；断层线下禁 `f32/f64` |
| I2 | 角度 | **BAM u16**（一圈 65536）；三角函数查表 |
| I3 | 随机 | PRNG(PCG32) 状态是 World 字段，随快照回滚；表现层用另一颗 |
| I4 | 顺序 | 一切遍历按池索引升序；确定性分配（存活掩码最低空位）；禁无序容器 |
| I5 | 协程 | 模拟协程状态在可 memcpy 扁平内存（自建字节码 VM，非 async） |
| I6 | 时间 | 固定 60 Hz，一切计时用整数帧 |
| I7 | 布局 | World 内无指针/引用/堆容器；实体互引用用 generation-index 句柄；**快照 = 整块字节复制** |

## crate 依赖图 + 子系统清单

```
  stg-godot(gdext) ─┐   [M2]         stg-ecl-compiler ──产出 EclImage──┐
  stg-py(PyO3)     ─┼── 依赖 ─► stg-core ◄──────────────────────────────┘
  stg-harness(CLI) ─┘   [M5]   （确定性内核）      stg-derive（proc-macro，被 core 依赖）
```

| crate / 模块 | 职责 | 状态 | 深度 |
|---|---|---|---|
| `stg-core/math` | 定点核 Fx/Angle + 查表三角/CORDIC/isqrt/easing + 烘焙表 | ✅ M0 | [fixed-point-corners](fixed-point-corners.md) |
| `stg-core/{bullets,shots,enemy,field,items,player}` | 五实体池（`define_pool!` 实例，池即层）+ 自机 | ✅ M0 | [pool-memory-layout](pool-memory-layout.md) |
| `stg-core/xform` | 弹变换段池（D4，会照剧本演的弹） | ✅ M0 | [xform-ops](xform-ops.md) |
| `stg-core/world` | WorldBody 字段所有权 + 各相位函数 + 写 API + 场界 + 双通道读出口（`view()` 通道 A / `take_requests()` 通道 B / `frame_events()`） | ✅ M0-M2前置 | [stg-world-design](../stg-world-design.md) Part III |
| `stg-core/tables` | `WorldTables` 数据层（shottype/道具/外观/角色）+ 规范字节 serde + content_hash | ✅ M0-17/**C11** | [本文子系统](#资产管线-c11) |
| `stg-core/consts` | 脚本可见引擎常量注册表（① 结构常量 / ② 表符号） | ✅ C14 | [ecl-lang](ecl-lang.md) |
| `stg-core/ecl` | 栈机 VM（协程池 256）+ op 表 + syscall 白名单沙箱 + EclImage + 安全绑定层 | ✅ M1 | [ecl-ops](ecl-ops.md) |
| `stg-core/{checksum,rng}` | vendored FNV-1a64 校验和 + vendored PCG32 | ✅ M0 | [checksum-mechanism](checksum-mechanism.md) |
| `stg-core/step` | **组装层**：§3.5 宪法顺序唯一持有者 + World{body,tasks} + 快照 | ✅ M0 | design_doc §3.5 |
| `stg-derive` | `#[derive(Checksum)]` + `define_pool!` proc-macro | ✅ M0 | — |
| `stg-ecl-compiler` | `.ecl` 表层语言（lex/parse/typeck/slots/codegen）→ EclImage；离线编译 | ✅ M1.9 | [ecl-lang](ecl-lang.md) |
| `stg-harness` | CLI：金向量对拍 + bench 基线 + 烘焙表 bake/verify | ✅ M0 | [bench-baseline](bench-baseline.md) |
| `stg-godot` | gdext WorldBridge + MultiMesh + 请求分发器 | ⏳ M2 未建 | design_doc §6 |
| `stg-net` / `stg-py` | UDP 会话/重同步 / PyO3 headless 并行 env | ⏳ M4/M5 未建 | design_doc §7 |

## step 流水线（相位 0–10，顺序即宪法，PhaseGuard 押运）

| 相位 | 动作 | 拥有者 | 备注 |
|---|---|---|---|
| 0 | `begin` | world | 清帧内输出缓冲 |
| 1 | `decode_input` | world | InputFrame → 自机意图 |
| 2 | `PH_DIRECTOR` → `run_tasks` + 导演闭包 | **组装层** | ECL 任务运行器是默认租户（P2 导演槽），跑在导演闭包**前** |
| 3 | `update_players` | world/player | 自机移动 + shottype 表驱动发弹 |
| 4 | `run_transforms` | world/transform | 变换段游标推进 |
| 5 | `integrate` | world/integrate | 运动积分（POLAR/CART 双表示）+ 道具磁吸 |
| 6 | `collide` | world/collide | 碰撞矩阵（体碰/受击/擦弹/拾取） |
| 7 | `settle` | world/settle | 结算三趟：伤害/死亡/入账/掉落 |
| 8 | `PH_ECL_HOOK` | — | ECL 事件挂钩槽（预留，暂空） |
| 9 | `cleanup` | world/cleanup | 越界/寿命回收 |
| 10 | `advance` | world | 帧号 +1 |

> `step()` 是 `step_with_director()` 的空导演特化。ECL 与"上层在相位间读事件后自行动作"共存——world **无回调**（P5）。

## 子系统骨架

- **数学核**：`Fx=Q16.16(i32)`、`Angle=BAM(u16)`，`repr(transparent)` 强类型。`mul` 走 i64 中间量 `>>16`；
  平方距离/点积保 Q32.32 于 i64 不归一化（累加器模式）。sin/cos 查 16384 四分之一波表，atan2 整数 CORDIC 钉 16 轮。
- **池框架**：`define_pool!` 生成 SoA + generation 句柄 + **存活掩码即分配器**（最低空位，无 free-list）。
  复用槽必须写满全字段（exhaustive `Init` 编译期强制）——支撑"校验和哈希全槽不掩码"。
- **世界层**：接口按 crate 级纪律书写（调用方只走安全读/写 API，永不碰池内存）。错误三铁律 P4：
  资源耗尽→确定性降级不 panic；调用方违约→安全结果+计数；引擎自身 bug→debug 帧内断言。
- **ECL**：自建栈机 VM（模拟协程状态可 memcpy，I5）。`EclImage` = 只读镜像（字节码 + 具名入口表 + content_hash）。
  syscall 白名单沙箱是 ECL 与世界的唯一接口。`.ecl` 表层语言离线编译成 EclImage，启动时装载。
- <a id="资产管线-c11"></a>**资产管线（C11）**：`WorldTables` owned 化，从**规范字节**（`from_bytes`，只读整数守 I1）加载，
  不再硬编进二进制。真 `content_hash`（FNV over body）焊死 coherence：`compile_for_table` 把表 hash 盖进
  `EclImage` → `World.tables_hash` → `start_main` 拒绝错配表（`TableImageMismatch`）。烘焙纪律：`tables_v0.bin`
  由 harness 生成、commit、CI 逐位对拍（同数学表）。
- **校验和 / 快照**：vendored FNV-1a64（算法字节冻结），字段级、哈希全槽、小端。`#[derive(Checksum)]` 防漏
  （新字段默认入校验）。快照 = World 整块字节复制（POD，全零合法）。CI/金向量逐帧对拍，联机随包 K=20 采样。
- **输入**：`InputFrame` 是唯一向下入口；动作输入抽象（非扫描码），来源统一（本地/回放/网络）走同一路径。
- **harness**：Phase 1 的驱动器——两段金向量符卡、bench 基线、烘焙表 bake/verify（允许浮点）。

## 数据流（一帧）

```
InputFrame ──► step ──► [相位 0-10 演化 World] ──► 通道A：状态视图（表现层读）
   ▲                         │                    └► 通道B：渲染请求队列（M2 分发给 MultiMesh）
   └── 回放/网络/本地同一路径  └── 校验和（逐帧对拍 / K=20 采样）
```

## 面向未来的接缝（下一阶段从哪接）

| 里程碑 | 接缝（已就位的焊点） | 还缺 |
|---|---|---|
| **M2 表现层**（gdext） | 可见性收口 ✅（写走 API）+ **通道 A `WorldView` ✅**（五池每字段裸切片 + `alive_words()` + `view()` 单入口）+ **通道 B ✅**（`RenderReq` + `emit_req`〔世界 API/syscall 27/`.ecl` 内建〕+ `take_requests()` 出口 + settle 敌死请求）+ **外接前收口 ✅**（快照哨兵防漏 + `tasks`/`rng`/`frame`/`events` 封口配 `frame()`/`frame_events()`/`tasks()` 读口 + `spawn_entry*` 表守卫 + `ENGINE_VER` + 场界常量 pub——断层线两出口齐备、误用面收干净） | 新建 crate（WorldBridge + MultiMesh + 请求分发器 + 定点→浮点边界）；开工先还 A1（道具 sprite 列）/A2（bench 重跑） |
| **M3 回滚 harness** | 快照 = memcpy（账已实测）；RNG 随快照回滚；回放头素材就位：`seed`（World provenance 字段，`seed()` 读回）+ `content_hash` + `engine_ver` | 环形快照缓冲 + 延迟/输入扰动/校验和风暴 harness |
| **M4 网络** | lockstep+rollback 模型；K=20 采样对拍；`engine_ver`/内容哈希握手 | `stg-net`（UDP + 会话/重同步）起 phase 2 |
| **M5 headless 并行** | 单 world 单线程、并行只在 world 之间（P3）；无外部依赖 | `stg-py`（PyO3 env） |
| 玩法小刀 | `FieldPool` 消弹区就位（bomb 首租户）；`Shooter.flags` bit0 预留 homing | bomb 铺一个 field；homing 转向率存放待拍 |

> 未做但已记档的技术债/扩展点见 [`docs/follow-ups.md`](follow-ups.md)（开工前先读）。

## 权威文档索引

| 文档 | 定位 |
|---|---|
| [`design_doc.md`](../design_doc.md) | 总纲 v0.3——架构/断层线/确定性契约/ECL/网络/测试（冲突时以世界层蓝图为准） |
| [`stg-world-design.md`](../stg-world-design.md) | 世界层实施蓝图 v1.0（20 轮评审拍板）——P1-P6 原则、A1-A9 架构、D1-D12 局部实现 |
| [`CLAUDE.md`](../CLAUDE.md) | 工作宪法：硬规则单一入口（断层线/不变量/原则/自检清单） |
| [`PROGRESS.md`](../PROGRESS.md) | 当前位置/下一步/里程碑史的**唯一权威** |
| `docs/*.md` | 专题速查：定点坑/校验和/池布局/变换 op/ECL 语言与字节码/性能基线 |
| `docs/superpowers/{specs,plans}/` | 各切片的设计 spec 与实施计划（历史记录） |
