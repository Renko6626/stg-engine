# M1 ECL VM + 编译器 —— 设计 spec

> 状态：已过 grill 拍板（2026-07-18，七项）+ 承接 design_doc §4 v0.3 草案与栈机调研落档
> （`2026-07-18-ecl-vm-route-research.md`）。定位：**Phase 1 第二支柱**——字节码 VM +
> 任务协程池 + syscall 绑定层 + Rust builder DSL，跑通一张彩虹风铃类环形符卡入金向量。

## 拍板纪要（grill，2026-07-18）

1. **指令编码：字流 + 元数表**——指令 = 1 头字（opcode 在低 8 位，余位留白）+ N 操作数字
   （N 由 `ARITY: [u8; 256]` 钉死，D4 同款设计语言）；立即数内联；解码 = 读头字→查表→定长跳。
2. **指令预算：双层 + 超限杀**——每任务 1024 条/帧 + 全局 65536 条/帧（按池索引升序消耗，
   I4——两机饿死同一批）；超限 = **确定性报错杀任务** + 事件 + diag 计数（死循环是作者 bug，
   响亮地死；与栈溢出同款处置）。
3. **容量**：求值栈 **32 字** / locals **64 字**（丙方案硬下限）/ 调用栈 **8 帧** /
   任务池 **cap 256**——Task ≈ 460B、TaskPool ≈ 118KB（World ~1.04MB，校验和 +12%）。
   全部编译期常量，金向量实测不够再调。
4. **locals 语义：任务全局共享**——sub 调用不开新窗口，`CallFrame = { ret_pc: u32 }`；
   入参走求值栈；递归共享 locals（作者须知入文档）；丙方案 xform 区间引用与 shooter 状态
   零歧义。将来要重入加"帧相对寻址"指令族，纯增量。
5. **rank/难度：脚本变量**——难度 = 开局写入 `globals` 约定槽的一个数（随快照/联机天然同步），
   脚本自己 `if`/进算式（连续难度）；VM 零支持；头字留白位可供未来 mask，不实现。
6. **DSL：builder + 结构化糖**——普通 Rust 函数 + 标签回填，闭包式结构块
   （`repeat(n, |s|…)` / `if_ge(…, |s|…)` / `sub`/`spawn` 糖）；糖层语义即未来表层语言
   编译器的后端，非丢弃件。proc-macro 不做。
7. **验收符卡：彩虹风铃类环形卡**——多重彩环（异速/异色环批量铺设）+ xform 变换（转向/
   加速环）+ boss 移动 + 符卡计时（boss_set），行为级复刻（非逐字节模拟 ZUN 二进制）。

## 承接既定（不重烤，来源注明）

- **栈机**；`wait` 让出即协程全部魔法；**`spawn_task` 次帧首跑**（born_frame 戳）；
  owner 轴自动死亡 / parent 轴默认不级联（detached-by-default）+ 显式 `kill_task/kill_children`
  （design_doc §4.2-4.3 v0.3 拍板）。
- **syscall 白名单** = D12 成员表；世界不认识任务（P1）：`create_bullet` 的 `task_script`
  参数在绑定层组合（先 create 拿句柄再 spawn_task）。
- **丙方案 ABI**：变换序列 = 本任务 locals 区间引用 `(xform_off, xform_cnt)`，每槽 3 字打包
  `word0=(wait<<16)|(op<<8)`、`word1/2=args`；绑定层解包成 `XformSlot` 数组调
  `create_bullet_with_xform`；越界确定性报错。
- **创建原语不内建 RNG**（路线甲）；随机散布 ECL 循环逐发。

## 结构落位

```
World {                        // 组装层（stg_core::step）
    body: WorldBody,           // 既有
    tasks: TaskPool,           // 新增：ECL 类型物理住组装层（依赖方向 ECL→world，world 无知）
}
crates/stg-core/src/ecl/       // 新模块（断层线以下）
    task.rs                    // Task + TaskPool（手写特例池，xform.rs 先例；Checksum 全量）
    vm.rs                      // 解释器：op 表 + ARITY + 预算 + 错误处置
    ops.rs                     // opcode 常量（编号即契约，冻结纪律同 D4）
    syscall.rs                 // syscall 号表 + 绑定层（&mut WorldBody + &WorldTables + 任务上下文）
crates/stg-ecl-compiler/       // 启用：builder DSL → EclImage
    EclImage { code: Vec<u32>, subs: Vec<u32/*入口*/>, content_hash: u64 /*占位同 WorldTables*/ }
```

- **运行时序**：ECL 任务运行器 = **相位 2 导演槽的默认租户**（P2 既定）；`step` 内先跑
  ECL 运行器（按池索引升序遍历任务：owner 门禁 → born_frame 门禁 → wait 递减门禁 →
  解释执行至让出/预算尽），再跑注入的导演闭包（harness/测试用，二者共存）。
- **EclImage 传递**：与 `&WorldTables` 同款——`step(world, tables, ecl: &EclImage, input)`
  参数穿线，不进 World；无脚本场景传空镜像（零任务即零成本）。
- **Task 字段**（§4.2 草案 + 本次校准）：`script: u16, pc: u32, wait: u16, born_frame: u32,
  owner: Handle(u32 语义), parent: TaskHandle, sp: u8, csp: u8, stack: [i32; 32],
  calls: [u32; 8], locals: [i32; 64]` + 池存活掩码。全量入校验和（P6）。

## 指令集 v1（编号即契约；十位=族号留空隙，D4 惯例）

- **0x：控制** `END=0`（任务正常完成）`WAIT=1`（栈顶帧数）`JMP=2` `JZ=3` `CALL=4` `RET=5`
- **1x：栈** `PUSHI=10`（立即数内联）`PUSHL=11`（读 locals[imm]）`POPL=12`（写 locals[imm]）
  `DUP=13` `POP=14`
- **2x：整数算术** `ADD/SUB/MUL/DIV/MOD/NEG = 20-25`（i32；除零确定性报错杀任务）
- **3x：定点/角度** `MULF=30`（Q16.16 乘，i64 中间量）`DIVF=31` `SINB=32` `COSB=33`
  （BAM 查表，走 math 核）
- **4x：比较** `EQ/NE/LT/LE/GT/GE = 40-45`（弹 0/1）
- **5x：任务** `SPAWN=50`（script id + owner 来源枚举操作数）`KILL_SELF=51` `KILL_CHILDREN=52`
- **6x：syscall** `SYS=60`（syscall 号内联 + 参数按号定义从栈取）——一切副作用唯一通道
- 未知 op / pc 越界 / 栈越界 / 预算超限：**确定性报错**（杀任务 + `EVT_TASK_FAULT` 事件 +
  diag 计数），绝不 panic（release）；debug 帧内断言照 P4-c。

## syscall 号表 v1（冻结起点；参数全 i32，Fx/BAM 语义按号约定）

读：`frame / player_x / player_y / self_x / self_y / self_hp / rand_range / get_var / last_status / nearest_enemy`
写：`create_bullet(丙方案 8 参) / create_bullets_batch / create_player_shot? 不入（自机弹归世界）
/ spawn_enemy / drop_item / set_var / boss_set / pulse_signal / move_enemy_to / 弹 setter 族
（speed/angle/vel/ang_vel/accel/gravity/turn/aim/stop）/ attract_all_items`
（`self_*` = owner 句柄解引用：owner 为敌读敌、为弹读弹；悬垂 → 任务已被门禁杀，不可达。）
**appearance 表**：WorldTables 新增 `appearances: &[AppearanceCfg{radius, sprite}]`——
`create_bullet` syscall 按 id 查表填默认、显式参数可覆盖（§3.2.1 既定）。

## 金向量与测试

- **金向量二号场景**（新 harness 子命令参数或独立入口 `golden --scene ecl`）：boss 敌人 +
  彩虹风铃卡脚本（DSL 转写：≥3 重异速彩环周期铺设 + xform 转向环 + boss `move_to` 巡游 +
  `boss_set` 计时递减 + 难度变量参与环密度算式）跑 600 帧逐帧校验和，进 CI 三平台对拍；
  一号场景（现 golden）保持不动。
- **判别式单测**（最小集）：解码器逐 op 元数走格；wait 让出/递减/唤醒帧精确；spawn 次帧首跑
  （born_frame 门禁判别）；owner 失效任务死；预算超限恰在第 1025 条杀 + 事件；栈溢出/除零/
  坏 op 确定性报错；CALL/RET 往返 + locals 共享语义；丙方案 xform 区间解包逐位；SYS 白名单
  逐号（坏号报错）；DSL 结构块（repeat/if_ge）生成跳转的边界回填。
- **VM fuzz 冒烟**（§9 承诺的最小版）：确定性 PRNG 生成随机字节码 N 份，断言只出确定性
  报错、无 panic、无越权（World 校验和只经 syscall 变化）。
- **变异检验 ≥3**：预算界 1024→∞（超限测试红）；born_frame 门禁删（次帧首跑测试红）；
  ARITY 表某 op 元数改（解码走格测试红）。

## 收尾义务

design_doc §4 草案落地括注 + §10 未决项关闭；stg-world-design D12 增补 syscall 号列；
`docs/ecl-ops.md` 新建（op/syscall 速查，xform-ops.md 同款）；PROGRESS 史行；
follow-ups：`spawn_task_now` worklist 版（既定后期）、帧相对寻址预留、表层语言。

## 验收

金向量二号三平台逐帧全等 + 一号不回归；判别单测全绿过变异；fuzz 冒烟零 panic；
clippy/fmt/防火墙零告警；World 新增 tasks 字段全量入校验和 + `copy_into` 拷贝（判别测试钉）。

> 修订（2026-09-24，引擎第二刀）：静态可判的非法指令改在加载时拒绝；两个预算合成一个倒数，
> 1025 边界不变。
