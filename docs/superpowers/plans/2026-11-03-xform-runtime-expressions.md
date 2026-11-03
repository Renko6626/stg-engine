# xform Runtime Expressions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为现有 xform 加入按值形参、运行时表达式和 `at_spawn` / `at_effect`，保持静态路径与现有效果语义。

**Architecture:** 保留现有 2048×16 槽运行段，以共享只读模板保存原始表达式。发射器自有固定绑定值；弹参数原地覆写，必要原值存段尾数据槽，每段仅增加 4 B 元数据。动态求值产生 raw 参数后调用原有操作；世界层只接收 core-owned xform 数据，不依赖 ECL、不保存回调。

**Tech Stack:** 现有 Rust workspace、Q16.16/BAM、现有 ECL 编译器和 headless harness；不新增依赖。

**Spec:** [2026-11-03-xform-runtime-expressions-design.md](../specs/2026-11-03-xform-runtime-expressions-design.md)。2026-11-03 用户在完成存储及 CPU 成本讨论后要求读代码、完善计划并直接实施；沿用设计稿的容量、表达式边界和新 ABI。当前分支原地顺序实施，由用户明确确认；不自动提交、推送或合并。

## Global Constraints

- 每段仍为 16 个物理槽，操作 M（含 STEP scratch）与数据区共用容量：`M + ceil(C/2) <= 16`，C 为出生后仍需读取的原值数。STEP 每条 2 槽；段池 2048，总弹池 8192。
- 沿用合计 8 个 i32 逻辑缓存上限，不拆出更大的绑定/出生配额；C<=8。每组合表达式最多 128 条指令、临时栈 32 个 i32；标记只位于完整参数外层，禁止嵌套。
- 发射器 XformBinding 为 36 B，段 XformSegmentState 为 4 B；全零为未绑定/旧静态状态。预计 World 新增 **44 KiB**，2048 RL 环境新增 **88 MiB**；所有尺寸须实测核对。
- 不挖单参操作的闲置 args，不覆盖 STEP scratch、LOOP 计数或 BOUNCE 等持续读取的参数；不把发射器捕获移进永久 Task.locals，不增加备用捕获池。数据槽头固定 END，不作为效果执行。
- P1/I7/P5：World 不 import `ecl::*`，无持久指针、堆容器、闭包或函数指针；不为表达式建 Task。
- 新增持久字段须覆盖完整初始化、checksum、SaveBytes、copy_into；ENGINE_VER 27→28。
- 保留旧静态入口、op 编号、@N 后置等待、相位顺序、冻结、delay、LOOP fired-region 和现有静态参数降级。
- 允许的 xform 体函数仅 global/sin/cos/atan2/dist/aim_player；普通实参保持现有 ECL 求值语义。动态外观校验不能省略。
- 当前分支 `codex/core-module-cleanup`，用户未跟踪产物 `dist/` 保留。执行时先按 using-git-worktrees 检查隔离状态，不覆盖其它工作，不自动提交、推送或合并。
- 本计划任务有顺序依赖，默认建议当前会话顺序实施；只有用户明确选择委派后才分派。接手者读取 spec 与本节，不扩展到共享池压缩、持续追踪、脚本身份修复或其它待办。
- 新增测试只覆盖本次核心行为和具体覆盖缺口；每项主行为只在下述所属层保留主测试，完成后集中跑相关检查。

## Review Focus

1. 同一模板在两发射器中用不同形参：互不覆盖；重绑、reset、创建任务退出不改变已有弹。Task 4 集成测试负责。
2. 动态 STEP 启动后世界值改变：目标保持；LOOP 重入才刷新，计数和原表达式不被覆盖。Task 2 运行器测试负责。
3. WAIT_SIGNAL 阻塞期间全局通道值改变：保持进入时的通道；恢复后再次进入才重采样。Task 2 运行器测试负责。
4. 动态外观的形/色分属不同阶段：保留各自采样结果，图集空格按既有规则拒绝，不能生成意外隐形弹。Task 2 运行器测试负责。
5. 段尾捕获和已采样等待槽经存档/恢复或段复用：结果可重演、无上一颗弹残留；捕获不需要时可以用满16槽，放不下时编译期报清操作/数据占用。Task 2 的恢复/复用测试与 Task 4 的布局测试分别覆盖独立风险。

---

## File Structure

| 文件 | 责任 |
|---|---|
| 新 `crates/stg-core/src/xform/expr.rs` | 受限表达式 IR、常量/直接读取/组合程序、构造期校验与错误类型 |
| 新 `crates/stg-core/src/xform/program.rs` | 共享模板、阶段依赖与数据槽映射、36 B 发射器 binding、4 B 段状态、临时捕获视图 |
| `crates/stg-core/src/xform.rs` | 保持现有槽池和 op 号；增加逐段 program_state，更新手写复制/哈希/存档 |
| 新 `crates/stg-core/src/world/xform_eval.rs` | 按临时 self 上下文读取状态，执行有限表达式，复用现有数学语义 |
| 新 `crates/stg-core/src/world/xform_bind.rs` | 绑定、出生采样、效果进入采样、参数物化和动态外观校验 |
| `world.rs`、`world/transform.rs`、`step.rs`、`lib.rs` | 接入现有创建/相位、清理进入标记、状态/版本约束 |
| `ecl/image.rs`、`ecl/shooter.rs`、`ecl/task.rs` | 镜像持有只读表；发射器自有 binding；任务复用/复制覆盖 |
| `ecl/syscall.rs`、新 `ecl/syscall/xform.rs`、`ecl/syscall/{bullet,shooter}.rs` | 新 ABI 的解包/上下文适配；复用旧建弹和任务附挂路径 |
| `lang/{ast,parse,xform_map,slots}.rs` | 可选形参、完整参数标记、操作类型信息、暂存区分配 |
| 新 `lang/typeck/xforms.rs`，及 `typeck/{checker,exprs,typed_ast}.rs`、`typeck.rs` | 模板签名、形参作用域、阶段与纯函数检查、带参引用 |
| 新 `lang/codegen/xform.rs`，及 `codegen.rs`、`codegen/builtin.rs`、compiler `lib.rs` | 共享程序发码、捕获缓存、静态/程序 ABI 选择、builder 接线 |
| 现有核心/编译器测试模块 | 与对应任务一起补核心行为；不建独立验证项目 |
| 新 `crates/stg-harness/src/xformbench.rs`，及 harness `main.rs` | 可选的等价轨迹表达式基准；默认 bench 不扩张 |
| `docs/xform-ops.md`、`docs/ecl-lang/{4-bullets,7-reference,8-errors}.md`、`docs/ecl-ops.md` | 用户语义、边界、ABI 和错误说明 |
| `stg-world-design.md`、`docs/{pool-memory-layout,bench-baseline}.md`、`PROGRESS.md` | 状态预算、实测性能、版本和交接记录 |

### Task 1: 共享程序类型与有限表达式求值

**Files:** Create `xform/expr.rs`、`xform/program.rs`、`world/xform_eval.rs`；Modify `xform.rs`、`world.rs`。测试放相应新模块的 `#[cfg(test)]`，不先改段池布局。

**Interfaces:**
- Produces `XformPrograms::try_new(Vec<XformProgramInit>) -> Result<Self, XformProgramError>`、`empty()`、`program_id(u16) -> Option<XformProgramId>`。
- Produces spec §4 的 `XformBinding`、`XformSegmentState`、`XformEvalOwner`、`XformCaptureView` 和表达式/读口类型。
- Produces crate-private `WorldBody::eval_xform_expr(&self, expr: &XformExpr, captures: XformCaptureView<'_>, owner: XformEvalContext<'_>) -> Result<i32, XformEvalError>`；Values 读取临时工作值，Segment 按共享映射直接读取尾部数据。
- `XformEvalContext` 仅用于本次调用：Caller(XformEvalOwner)、Spawn(&BulletInit)、Effect(BulletHandle)；不写入 World。

- [ ] **Step 1: 记录实施前基线。** 在执行工作区记录 HEAD、`git status --short`、`cargo run --release -p stg-harness -- bench --frames 600` 输出及机器/构建模式；保存到执行记录，不提交巨量原始输出。
- [ ] **Step 2: 增加有限程序的核心测试。** `xform_expr_numeric_and_short_circuit`：断言 `3fx*0.5fx=98304 raw`、`-7/3=-2`、BAM cast/三角读口及 `0 && (1/0)` 不求右侧；`xform_program_rejects_invalid_shape`：各用一项拒绝数据捕获下标越界、跳入数据区非终止位置、反向表达式跳转和超 32 栈；不穷举垃圾字节。
- [ ] **Step 3: 运行新测试确认缺少生产实现。** `cargo test -p stg-core --lib xform_`，预期接口缺失/新断言失败；此命令不替代末尾完整相关检查。
- [ ] **Step 4: 定义共享 IR 和阶段布局。** 模板保留两个语义外观参数；逻辑工作缓存按形参、Bind、Spawn 排列，最多 8。效果持久读取改用紧凑数据下标，模板保存逻辑字到尾槽的映射和合法可执行边界。Effect 参数保留表达式引用；表达式支持常量、直接读取和有限组合程序，只有前向短路分支。
- [ ] **Step 5: 实现读取和求值。** 对齐 spec §3.3 的数值和 owner 矩阵，按实际读取延迟获取 CART 极坐标；绑定 self_age 用调用任务 born_frame，Spawn/Effect 用弹 born_frame。复用现有 Fx/CORDIC/isqrt/trig，不调用 ECL VM 或 syscall 派发，不新增通用算术框架。
- [ ] **Step 6: 验证。** `cargo test -p stg-core --lib xform_` 与 `cargo test -p stg-core --lib world::motion::tests` 通过；检查新模块无 ECL 类型导入。Task 1 交付可构造、可求值的 core 程序，尚不接入逐帧状态。

### Task 2: 捕获状态、出生/效果采样与回滚

**Files:** Create `world/xform_bind.rs`；Modify `xform.rs`、`world.rs`、`world/transform.rs`、`step.rs`、`ecl/shooter.rs`、`ecl/task.rs`、`lib.rs`、`ecl/image.rs`、compiler `lib.rs`，以及所有 `ImageParts` 初始化处。更新 spec 列出的状态预算文档。

**Interfaces:**
- Consumes Task 1 的程序/表达式。
- Produces spec §5 的 `WorldBody::bind_xform(...) -> Result<XformBinding, XformEvalError>`、`create_bullet_with_bound_xform(...) -> BulletHandle`。
- Produces `WorldBody::run_transforms(&mut self, &XformPrograms, &WorldTables)`；私有 helper `resolve_xform_slot(i, programs, tables) -> Result<XformSlot, XformEvalError>`，统一负责当前槽进入采样。
- Produces `ImageParts.xforms`、`EclImage::xforms()`、`ImageBuilder::set_xform_programs()`；镜像不含 World 状态。

- [ ] **Step 1: 增加阶段判别测试。** `xform_samples_bind_spawn_and_effect_at_distinct_times`：同一读口在三个时点分别设为 1/2/3，延迟后分别观察 1/2/3；捕获形参 `extra=0.5fx` 加出生速度 `2fx` 得 2.5fx，之后把弹速改为 5fx 不改变出生缓存。
- [ ] **Step 2: 增加状态型效果测试。** 扩展已有 STEP/LOOP 用例，目标从 4fx 改成 8fx 时当前 STEP 仍落到 4fx，重入后才落到 8fx；循环次数保持原值递减。`xform_wait_signal_samples_once_per_entry`：先等通道 0，中途改全局为 1，脉冲 1 不放行，脉冲 0 放行；重入后可等 1。
- [ ] **Step 3: 增加动态外观与相位测试。** 复用已有合成图集，验证出生捕获形状、效果读取颜色，拒绝无效完整组合；部分设沿用只查轴的规则。相位测试使用 `high_speed=4fx` 的现有表 fixture，玩家从 x=10 移到 x=14，出生/效果采样分别得到 10/14；不启动浏览器。
- [ ] **Step 4: 扩展现有状态恢复测试。** 在已有 save/load/copy 测试中加入非零段尾捕获、进行中 STEP、停驻 WAIT_SIGNAL，恢复后同输入逐帧 checksum 一致；扩展段复用用例，确认旧程序元数据和尾槽值不泄漏，第一数据槽的 END 不执行 payload。不要新增整套回放测试工具。
- [ ] **Step 5: 运行上述相关用例确认失败，再加入布局。** `XformSegPool` 增加 `[XformSegmentState; 2048]`，`ShooterSlot` 增加36 B binding。段中不加固定捕获数组；同步 alloc/复用/default/copy/手写 checksum/SaveBytes，不改 `Task` 本体的布局。
- [ ] **Step 6: 实现绑定和出生物化。** 所有出生采样读取同一初始 BulletInit，在临时八字工作区求完；可直接使用的值写原操作参数，只把模板选出的 C 个原值写段尾。尾槽头固定 END，第一数据槽兼作终止位置，其余未用字写零。静态创建完整清空4 B program_state，不留下旧数据。
- [ ] **Step 7: 接入效果进入求值。** 保留静态槽快速分支；捕获从数据区直接读，不恢复整份 binding；先解析全部参数再覆写对应 raw 字，STEP tick 只读本次结果。维护 sampled_slot_plus_one，WAIT_SIGNAL 停驻不重采样，步进/回跳/终止清零；不重新物化 LOOP 计数，不回收持续效果仍读的槽。复用原 fire_op 和 fired-region 语义。
- [ ] **Step 8: 接入中立只读表与外观校验。** EclImage 默认空表；step 只传中立表到相位 4，World 不认识 ECL。外观运行校验对齐现有 atlas 规则，不改变静态旧路径。
- [ ] **Step 9: 更新版本并验证状态账。** ENGINE_VER 27→28；运行 `cargo test -p stg-core --lib world_size_sentinel -- --nocapture`，核对预计 WorldBody **1,078,808** / World **1,328,720**、xform池 **401,664**、ShooterSlot **80** 后更新哨兵及44 KiB预算，不能直接接受异常尺寸。
- [ ] **Step 10: 验证。** `cargo test -p stg-core --lib`、`cargo test -p stg-ecl-compiler --lib` 通过。检查 `ImageParts` 新字段在所有消费者编译通过，恢复和复用用例覆盖真正的非零数据。

### Task 3: ECL 绑定和发弹 ABI

**Files:** Create `crates/stg-core/src/ecl/syscall/xform.rs`；Modify `syscall.rs`、`syscall/bullet.rs`、`syscall/shooter.rs`、`syscall/tests.rs`，必要时在 compiler `lib.rs` 增加手写测试发码所需的薄 builder 方法。

**Interfaces:**
- Consumes Task 2 的 bind/create 接口、镜像表、ShooterSlot.binding。
- Produces `SYS_CREATE_BULLET_PROGRAM=202`、`SYS_SH_XFORM_PROGRAM=653`；参数次序严格按 spec §5。
- 旧 200/201/650 继续接受旧布局；新入口只由编译器选择，不暴露新的脚本函数名。

- [ ] **Step 1: 增加新 ABI 的实际行为测试。** 手写镜像调用 653 绑定两个不同值，再用 660 发弹；读取弹速确认两槽隔离。202 的形参和 task_script 顺序用不同的可观察值区分，成功仍附挂既有子任务。
- [ ] **Step 2: 增加一种绑定替换测试。** 653 后调用 650 或 reset 不继续使用旧程序；已有弹保持出生捕获值。非法 argc/program 在建弹之前拒绝，不产生半颗弹；只覆盖这条新边界，不复制旧入口的所有负例。
- [ ] **Step 3: 运行 `cargo test -p stg-core --lib ecl::syscall::tests` 确认新行为未实现。**
- [ ] **Step 4: 实现新派发与解包。** 更新号表、白名单和 dispatch；args_off/argc 立即消费，owner 通过句柄与 born_frame 适配为 core 数据，不向 World 传 Task。先形成完整 binding，成功才替换对应 shooter。
- [ ] **Step 5: 复用发弹逻辑。** 在原单发/发射器创建分支选择 legacy 或 bound API，保留既有角度网格、出弹偏移、资源耗尽短路、请求及任务附挂顺序；避免复制整个 sys_sh_fire 或 sys_create_bullet。
- [ ] **Step 6: 验证。** `cargo test -p stg-core --lib ecl::syscall::tests`、`cargo test -p stg-core --lib world::transform::tests` 通过；旧 syscall 的元数和可观察行为断言保留。

### Task 4: 表层语言、类型检查和程序发码

**Files:** Create `lang/typeck/xforms.rs`、`lang/codegen/xform.rs`；Modify File Structure 中列出的 compiler 文件。核心集成测试放新 `lang/codegen/xform.rs` 的测试模块，复用现有编译/World 驱动方式；不反向增加 core→compiler dev 依赖。

**Interfaces:**
- Consumes Task 1–3 的 XformPrograms、builder 附表方法和新 syscall ABI。
- AST `XformDef` 增加 `params: Vec<(String, Ty)>`。
- Typed AST 增加 `TypedXformRef { name: String, args: Vec<TypedExpr> }`；`CallArg::XformRef(Option<TypedXformRef>)`。`TypedInfo` 保存已判型模板；`TypedSub.xform_refs` 仍是去重名字表。
- `SubSlots.xform_regions` 改为 `BTreeMap<String, XformRegion>`：`Legacy { off: usize, cnt: usize }` 或 `ProgramArgs { off: usize, argc: usize }`。前者仍占 3×物理槽数，后者只暂存形参。
- program id 按模板名升序分配；Gen 持有名字到 id 的映射。已有全常量零形参模板不进入程序表，保持原 staging 和 ABI。
- 新模板在槽分配前产出持久集合、数据起点和下标映射；不改变原操作/STEP 的槽号。运行段将必要原值追加到段尾，不能将数据位置作为普通操作边界。

- [ ] **Step 1: 增加前端测试。** 可选形参、`NAME(args)`、两个求值标记可解析；参数类型/数量错误、未捕获的调用者局部名、嵌套标记、超过8逻辑缓存、禁用调用均给对应源码位置诊断。扩展已有槽容量用例：M=16/C=0 放行，M=14/C=3 放行，M=15/C=3 拒绝并报告17槽；ProgramArgs 仍只占立即传参区，求值栈峰值包含引用实参表达式。
- [ ] **Step 2: 增加真实脚本集成测试。** `xform_two_shooters_capture_independently`：绑定值 1fx/3fx，修改创建者局部为 9fx，再发弹仍分别 1fx/3fx；创建任务结束后，延迟的效果仍可执行。`xform_fire_and_shooter_share_stage_semantics` 让单发和发射器分别使用同一模板，观察一致的采样结果。
- [ ] **Step 3: 运行 `cargo test -p stg-ecl-compiler --lib xform` 确认新语法/行为失败。** 保留现有静态 xform 测试，不能用改宽预期处理兼容性失败。
- [ ] **Step 4: 实现 AST、作用域和类型检查。** 复用 `Checker::type_expr` 与 `type_rules`；先识别外层标记，再检查函数白名单。const 仍常量折叠；新动态参数检查操作的数值类型；旧静态 raw 写法不重解释。把用户定义与标记保留名冲突也变成编译诊断。
- [ ] **Step 5: 构造模板和有限表达式。** 生成 Bind/Spawn 初始化表及 Effect 引用；用阶段依赖确定 C，不做运行时分配或最后使用点回收。用布局单测确认只供出生使用的 extra 不进入数据区、后续 Effect 多次引用的同一 extra 只保留一份；混合阶段形/色保留仍需组合的固定值。重映射效果代码的捕获下标，检查 M+ceil(C/2)，不挖闲置 args；表达式仍保留短路语义。
- [ ] **Step 6: 调整槽分配与 builtin 降低。** 静态引用仍压 off/cnt；程序引用先按顺序求实参并写临时区，再压 program_id/off/argc，选择 202 或 653。不得让暂存区成为 shooter 的持久引用。同步所有 CallArg 匹配和栈峰值计算。
- [ ] **Step 7: 验证。** `cargo test -p stg-ecl-compiler --lib`、`cargo test -p stg-core --lib ecl::syscall::tests` 通过。对 static fixture 比较公开 `code()` 和空程序表，证明仍走原路径；动态端到端测试断言实际弹状态，不只断言发码常量。

### Task 5: 使用文档、等价场景基准与收口验证

**Files:** Create `crates/stg-harness/src/xformbench.rs`；Modify harness `main.rs`、必要的 builtin doc/元数据生成描述，以及 File Structure 中的文档。无新增 CI/Godot/Python 验证环境。

**Interfaces:**
- Consumes完整语言和运行器。
- Produces `xformbench::cmd_xform_bench(frames: u32) -> ExitCode`，由 `bench --xform-expr --frames N` 选择该专用模式；默认 bench 的场景和时长不变。

- [ ] **Step 1: 写作者参考。** 在 `4-bullets.md` 写完整可编译示例、三个时点、self口径、8逻辑缓存上限、持久原值占用数据槽以及 STEP/WAIT_SIGNAL 行为；在 `xform-ops.md` 写操作/数据容量公式和外观规则；在 7/8 两篇及 ecl-ops 写签名、诊断、新 ABI。更新 builtin doc 时运行已有 `gen-ecl-meta` 同步生成段和 JSON，不顺带重做编辑器。
- [ ] **Step 2: 写可选性能场景。** 在 2048 弹、相同初始状态和触发节奏下比较旧常量路径、`at_effect(global(2) as fx)`、`at_effect((global(0)+global(1)) as fx)`；三组全局统一设为 1/1/2，目标均为 2fx。组合路径保留两次动态读，不能被全部折叠为常量。再比较出生求值的等量建弹成本、同目标持续 STEP 稳态成本。
- [ ] **Step 3: 在计时外验证基准有效性。** 固定采样帧检查弹数/速度/坐标及 fault/viol 为 0；使用生产编译和执行路径，建场、编译不计入模拟时间。不新增永久性能阈值断言，不把 checksum 不同误认为轨迹不同。
- [ ] **Step 4: 集中运行已有相关检查。**

```bash
cargo fmt --all --check
cargo test -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl
cargo clippy -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl --all-targets -- -D warnings
cargo test --release -p stg-core --lib world::transform::tests
cargo test --release -p stg-core --lib xform_
cargo run -p stg-harness -- verify-tables
cargo run --release -p stg-harness -- golden --out /tmp/stg-xform-v28-golden.txt
```

预期：全部命令成功，表未改变，golden 可生成。新旧版本因新增全量状态而 checksum 不同是预期；跨平台一致性由现有 CI 在后续正常交付时验证，此处不宣称本机已验证其它平台。

- [ ] **Step 5: 性能只在完整改动后集中量一次。** 跑 Task 1 同命令的改后基准，再跑 `cargo run --release -p stg-harness -- bench --xform-expr --frames 600`。专用模式同进程交错三轮，报告 p50/p99、触发频率、创建和稳态分别的开销；明显超出轮间波动才追加归因。结果写 bench-baseline，不承诺预设提升。
- [ ] **Step 6: 自查需求覆盖和变更范围。** 对照本计划 Review Focus 确认各自所属测试；检查 World 无 ECL 导入/引用，静态路径不进入求值器，更新实际内存账、ENGINE_VER、PROGRESS 和相关设计说明。记录未运行 Godot、Python、真实训练和其它平台。
- [ ] **Step 7: 交付审阅。** 总结实际修改、命令/结果和剩余限制；不自动提交、推送、合并，不仅为了泛化潜在风险追加机制或反复全量复审。执行中如需越过 spec 的容量/边界/协议，先向用户说明新决策。

## Plan Self-Review

- spec §3 的三个时点、形参隔离、短路和 self 语义分别归 Task 1/2/4；等待、缓动、循环及外观归 Task 2；静态兼容与新旧 ABI 归 Task 3/4。
- spec §4/6 的8逻辑缓存、阶段依赖和尾部数据槽、有限程序、36 B发射器绑定、4 B段状态、复制/存档/版本和失败语义归 Task 1/2/3/4；不增加固定弹捕获数组、备用池或异步恢复流程。
- 新接口的名称、参数次序、编号、计数和 ProgramArgs 与 spec 一致；编译器引用暂存区只立即消费。
- 五项 Review Focus 各只有一个主覆盖层；最后任务负责文档、有效基准和集中已有检查，不重复建立行为测试层。
- 执行状态与实际验证见下方记录；草案中的测试按真实实现合并为所属层的关键行为用例，不机械增加重复覆盖。

## Execution Ledger

- 用户已确认当前开发分支继续；现有 dist/ 产物保留。
- 代码调查：静态 xform 沿用 locals staging；新路径在引用处立即消费实参。编译器复用既有类型规则，core 使用独立受限 IR；效果入口才求值，等待和 STEP tick 不重复解释表达式。
- 测试按用户约定集中验证；只补关键行为和真实缺口，不机械执行逐任务全量检查或新增重复断言。执行日志位于 /tmp/sunyunbo/stg-xform-runtime/。
- Task 1→2：中立 IR、捕获视图、owner 上下文先完成，再连接段池；Task 2→3：绑定/创建 API 由 syscall 消费；Task 3→4：202/653 ABI 参数顺序一致；Task 4→5：完整生产路径用于等价轨迹基准。

- 实施接口调整：bind_xform额外接收WorldTables，以便绑定阶段已确定的外观值立即校验；保持core-owned边界，未增加持久状态或依赖。纯表达式IR降低与xform判型同文件，codegen/xform.rs负责立即实参暂存；减少编译器内部中间类型。
- Task 1：有限IR、控制流/栈校验及定点/短路测试通过。
- Task 2/3/4：运行时与编译器已接入，核心阶段/STEP/LOOP/WAIT_SIGNAL/保存恢复/复用测试通过；完整已有检查正在集中执行。

- 独立审阅修正：同模板嵌套实参先全部求值驻栈，再逆序写暂存区，栈深计算同步；混合阶段set_sprite的已知单轴在编译期校验。两个具体复现用例已观察失败后修复。出生创建仅向旧校验器传实际操作/数据槽，避免重复扫描清零尾部。


### 最终交付记录（2026-11-03）

五项任务均已完成，保留当前分支未提交改动。关键产物：core受限IR/程序表与阶段采样，36 B发射器绑定、4 B段状态和尾槽持久值，202/653内部ABI，带参模板与完整参数标记，参考文档与可选等价基准。

实际检查：

- `cargo test -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl`：core与compiler及相关集成检查通过；harness两项本地端口测试被沙箱拒绝。获准沙箱外运行`cargo test -p stg-harness -p stg-rl`后全部通过；没有删除、跳过或放宽有效断言。
- `cargo clippy -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl --all-targets -- -D warnings`、`cargo fmt --all --check`、`git diff --check`通过。最后扩展既有WAIT_SIGNAL恢复测试以覆盖LOOP重入通道重采样，并在既有数学测试加入三角/cast断言；受影响core xform检查和clippy再次通过。
- `cargo test --release -p stg-core --lib xform_`、`cargo test --release -p stg-core --lib world::transform::tests`通过。
- `cargo run -p stg-harness -- gen-ecl-meta`已同步文档与编辑器JSON；`cargo run -p stg-harness -- verify-tables`通过，烘焙表未改变。
- `cargo run --release -p stg-harness -- golden --out /tmp/sunyunbo/stg-xform-runtime/golden-v28.txt`完成三场景。本地不宣称跨平台对拍，新增字段使旧/新checksum不同。
- 改前/改后`bench --frames 600`与最终`bench --xform-expr --frames 600`完成；静态任务档另做一次同机新旧二进制交替归因。实际数据及限制见`docs/bench-baseline.md`动态xform节。

独立只读审阅提出的两项均已修复：嵌套同模板暂存覆盖、混合阶段外观固定轴漏检。两用例先失败后通过，无待修重要问题；不扩修既有镜像身份哈希缺口，不添加请求恢复、额外捕获池或逐帧依赖跟踪。接口适配仅增加bind_xform的中立WorldTables参数，以落实已批准的首次可确定外观校验。

未运行Godot/Python wheel、真实训练、浏览器交互或其它平台；未提交、推送、合并。现有dist/保留。

### 阶段性提交与集成

2026-11-03用户在实装交付后追加授权：阶段性提交并推送开发分支，合并并推送到main。此指令覆盖本计划实施期间不自动提交、推送或合并的限制；保留dist/本地产物，不纳入提交。集成前复跑core/compiler/harness/RL相关测试及格式检查；具体提交与远端状态以Git历史为准。
