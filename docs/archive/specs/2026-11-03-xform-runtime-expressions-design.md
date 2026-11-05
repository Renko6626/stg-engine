# xform 运行时表达式与分阶段求值设计

日期：2026-11-03。状态：已确认总体结构、示例语义，以及 Astra 评估后推荐的混合存储方案：发射器独立绑定，弹参数原地覆写，必要捕获保存在段尾。2026-11-03 用户在讨论存储和 CPU 成本后授权按本稿实施；实施及验证进展见计划执行记录。

2026-11-04扩展：用户已确认三阶段表达式中的rand支持，见[rand设计](2026-11-04-xform-rand-design.md)。本稿“禁止rand/纯表达式”描述ENGINE_VER28初始交付；ENGINE_VER30允许rand，其余写操作与任意sub仍禁止。

实施计划：[2026-11-03-xform-runtime-expressions.md](../plans/2026-11-03-xform-runtime-expressions.md)。

## 1. 已确认的目标与路线

保留现有 2048 段、每段 16 个物理槽的 xform 执行器。新增共享只读表达式和绑定模板；发射器独立保存按值捕获的参数。子弹把可直接使用的出生/效果采样结果写回操作参数，只将以后仍要读取的原始值保存在段尾数据槽。每段仅增加 4 B 程序/进入元数据，不增加固定八字捕获数组。动态参数求成具体数值后，仍调用现有效果实现。

`at_spawn`、`at_effect` 是语言的求值标记，编译为数据。World 中不保存 Rust 闭包、函数指针、引用、AST 或 ECL Task；不派生挂弹任务来模拟表达式。

```text
xformdef REAIM(extra: fx) {
    @60 add_speed(0fx);
    set_speed(at_spawn($self_speed + extra));
    set_angle(at_effect(
        atan2($player_y - $self_y, $player_x - $self_x)
    ));
}
sh_xform(0, REAIM(0.5fx));
```

不同时实施完整程序共享和段池压缩，不开放 xform 内的任意 sub、语句块、循环语法或每帧持续绑定，不扩充 `batch` 的表层签名，不迁移训练卡池和原作导入器。

## 2. 方案取舍

| 方案 | 取舍 | 选择 |
|---|---|---|
| 发射器和弹段各增加固定八字捕获区 | 实现直接，所有效果仍可用满 16 槽，但每 World 固定新增 108 KiB | 不采用 |
| **发射器固定绑定区；弹参数覆写；必要原值放段尾** | 每 World 新增约 44 KiB；需要持久捕获的动态模板减少可用操作槽 | **已确认采用** |
| 再挖用各操作闲置的 args 字 | 可追回部分操作槽，但不再减少固定 World 尺寸；需维护逐操作的存储 ABI | 本轮不做 |
| 发射器捕获放 Task.locals | 可进一步缩小新增状态，但占用 64 词预算，涉及动态槽号、跨 sub/跨帧所有权 | 本轮不做 |
| 全部迁移到共享程序和精简逐弹状态 | 有机会减少整段复制，但需改写段池、静态创建入口和已有诊断场景 | 本轮不做 |

## 3. 求值语义

### 3.1 三个时点

| 形式 | 求值时点 | self 含义 |
|---|---|---|
| `NAME(args)` 的实参 | 执行 `fire` / `sh_xform` 那条语句时，按声明顺序各算一次 | 调用者，沿用普通 ECL 表达式语义 |
| xform 中未标记的动态参数 | 本次绑定时算一次 | 调用者 |
| `at_spawn(expr)` | 每颗弹的最终出生位置、方向、速度确定后，第一条效果执行前 | 新弹的初始状态 |
| `at_effect(expr)` | 每次进入该操作、执行效果前算一次 | 当前弹的当前状态 |

顶层 xform 声明只看得见 const、形参和允许的状态读口，不能直接引用调用者的局部名字。创建者的局部变量、敌人共享槽、随机结果等可在 `NAME(args)` 的实参中先求值，再按值传入。

`sh_xform(0, X(a))` 后修改 `a` 不改变此绑定；`sh_xform(1, X(b))` 不覆盖槽 0 的值。随后创建的每颗弹都取得独立运行状态和仍需保留的原值。出生后再 reset、重绑发射器或结束创建任务，不改变已有弹。

首版把标记限定在整个操作参数上：`at_effect(expr + p)` 合法；标记嵌套或 `at_spawn(expr) + value` 给明确编译错误，提示把完整表达式放入标记。标记不提供普通 sub 中可调用的运行时函数。

### 3.2 操作时序

- 保持 `@N` 的后置等待、出现延迟、场景冻结以及 `tick_steps → wait → cursor` 的现有次序。
- ECL 发弹在相位 2，自机移动在相位 3，效果求值在相位 4；同帧的出生/效果读取可以不同，不统一成帧头快照。
- 一条操作的所有参数先求值，再执行操作。后面的同帧操作可读到前一操作改变后的弹状态。
- STEP 启动时采样目标和打包的时长/曲线，后续帧只读本次保存值；再次进入该 STEP 才重新采样 `at_effect`。
- `WAIT_SIGNAL` 进入槽时采样通道，阻塞等待期间不重复求值。放行后清除进入标记；LOOP 重入时重新采样。
- LOOP 重入只刷新动态参数，不重置其已有计数，也不改变 fired-region 的现有扫描规则。

### 3.3 首版表达式范围

复用 `int` / `fx` / `angle` 的现有类型规则、cast、算术、比较、一元运算和短路逻辑。xform 体的函数白名单为 `global`、`sin`、`cos`、`atan2`、`dist`、`aim_player`；允许现有 `$` 引擎变量。禁止体内的写操作、`rand`、任务插值槽、任意 sub 和其它 syscall。

`$player_x/y` 保持读取 1P 的既有口径；`aim_player()` 保持最近可瞄自机、无人可瞄时回退 1P 最后位置的查询口径，不改变原 `aim_player(offset)` xform 操作的无人可瞄 no-op 行为。

绑定阶段的 `$self_age` 维持调用任务龄；出生/效果阶段改为弹龄 `frame - bullet.born_frame`，按 wrapping 计算。弹的 hp/hp_max 为 0，self_enemy 为 -1；速度/角度通过现有极坐标读取规则取得，不读 CART 模式下的陈值。

旧的零形参全常量声明继续走原常量路径，保留其已接受的 raw 参数写法与编码；不借本次功能追改旧静态类型兼容性。新动态参数按操作的数值语义检查类型；STEP 第二参数仍为打包的 `int`，`@N` 仍只接受原有 u16 字面量。

## 4. 表达式、模板与状态

### 4.1 共享只读数据

新模块 `stg_core::xform::{program, expr}` 定义 `XformPrograms`、`XformProgramId`、`XformProgramInit`、`XformExpr`、`XformRead` 和 `XformEvalError`。编译器依赖这些 core 类型，World 不依赖编译器或 `ecl::*`。

`XformPrograms` 由 `EclImage` 持有。`step_with_director` 将 `ecl.xforms()` 和已有 `tables` 传给变换相位；不改变公开 step 参数列表。无脚本/旧静态镜像持有空表。

模板保存操作物理槽、形参数量、绑定/出生初始化表、参数来源以及段尾布局。绑定/出生求值使用逻辑缓存下标，最多八字；效果阶段的持久读取使用紧凑的段尾下标。模板保存两者映射，效果代码直接读取对应数据字，不需要每次重建完整八字缓存。原始表达式只在模板中保存，不能被首次求值或 LOOP 写回覆盖。

对于 `set_sprite(shape, color)`，模板保留两个语义参数再折叠成 raw sprite。形/色分别在不同阶段采样时，之后仍需参与组合的固定语义值放数据区，不从已折叠的 sprite 反向恢复。`set_shape` / `set_color` 的 args[1] 仍为表派生 stride，不当作可借用空间。

表达式常量先折叠；单个状态读口/缓存读口保留直接读取形式；组合表达式编译为小型指令程序。程序无反向跳转、无 CALL/SYS/WAIT；前向跳转仅用于短路逻辑。构造表时校验读口、缓存下标、前向目标、路径栈平衡和一个返回值。

### 4.2 捕获位置与操作容量

编译器做阶段依赖分析，不做跨操作的最后使用点回收：

1. 普通参数或出生结果若已完整物化为 raw 参数，以后只需执行该数值，就写回原操作槽。
2. 只有出生后仍被效果表达式引用的绑定原值，以及混合阶段外观组合仍需读取的语义值，进入持久集合。
3. 同一逻辑捕获被多次引用，只在持久集合中保存一份。形参只用于 `at_spawn($self_speed + extra)` 时，出生后可丢弃 extra；若还用于 `at_effect(global(0) as fx + extra)`，则保留原始 extra。
4. 持久集合在段尾固定存放，整段生命周期不迁移。原操作 args、STEP scratch、LOOP 计数及 BOUNCE 后续仍读的墙掩码不借给其它捕获。

设操作及 STEP scratch 共占 **M** 个物理槽，出生后需保留 **C** 个 i32 原值：

```text
M + ceil(C / 2) <= 16
[操作及 STEP scratch ……][数据槽 ……]
                           每槽 args[0..2] 保存两个原值
                           wait=0, op=END, pad=0
```

首个数据槽同时承担自然 END，不额外再占一个终止槽。C=0 时仍可使用全部 16 槽；C=1～2 时最多 15 个操作物理槽；C=8 时最多 12 个。布局超限时编译器分别报告操作、捕获字和所占数据槽，不添加备用池、堆分配或自动改用空闲 args。

数据区由模板显式定义。动态程序的 LOOP 目标只能是操作边界或合法终止位置，不能进入 scratch 或数据区其它位置；不能沿用旧 validator 将所有尾槽一概视为操作边界的推断。第一格数据槽的 END 头可以作为自然终止位置，payload 不作为操作执行。

### 4.3 容量提案与内存账

供审阅的首版固定上限：

- 沿用此前合计 **8 个 i32 逻辑缓存值**上限，包括声明的形参、未标记动态参数的绑定采样、`at_spawn` 的出生采样；不拆成两套更大的上限。发射器保存绑定值和零初始化的出生位置，创建时用临时八字工作区完成出生计算；仅 C 个仍需保留的原值写入弹段。直接引用形参不额外占逻辑缓存，C 不超过 8。
- 每个组合表达式最多 **128 条指令**，临时求值栈最多 **32 个 i32**；求值栈不持久化。
- 每个程序的操作、STEP scratch 和数据区合计最多 16 个物理槽；STEP 每条 2 槽；LOOP/END 不新增表层语法。程序编号是 u16，最多 65535 个程序。

仅发射器使用的 `XformBinding` 为 36 B；每段使用独立的 `XformSegmentState`，为 4 B。两者均使用 `#[repr(C)]`：

```rust
struct XformBinding {
    program_plus_one: u16,   // 0 = 无程序；有效 id + 1
    reserved: [u8; 2],
    captures: [i32; 8],
}

struct XformSegmentState {
    program_plus_one: u16,
    sampled_slot_plus_one: u8, // 0 = 未进入；WAIT_SIGNAL 等待期间保持
    reserved: u8,
}
```

全零分别是无绑定、旧静态段状态。捕获数量、数据起点和映射属于共享模板，不在每实例重复保存。所有 reserved、未用字和发射器中暂未出生采样的位置写 0。

| 新增位置 | 增量 |
|---|---:|
| `XformSegPool.program_states[2048]` | 8,192 B，8 KiB |
| 每任务 4 个 `ShooterSlot` 各增加一个 binding，256 任务 | 36,864 B，36 KiB |
| 每个 World 合计 | **45,056 B，44 KiB，约增加 3.5%** |
| 16 份 World 快照的额外预算 | 0.6875 MiB |
| 2048 个 RL 环境的额外 World 预算 | **88 MiB**，不含其它训练数据 |

当前尺寸基线为 WorldBody 1,070,616 B、World 1,283,664 B、ShooterSlot 44 B、xform 池 393,472 B。预计新增后分别为 WorldBody **1,078,808 B**、World **1,328,720 B**、ShooterSlot **80 B**、xform 池 **401,664 B**。相比原固定缓存提案，每 World 少 64 KiB，2048 环境少 128 MiB；这不是对当前引擎的净节省。以上均为布局推算，实施后用 `size_of` 实测核对。

共享只读表达式另占镜像内存，可由多个 RL 环境共享。原 384 KiB 槽区保留，其中一部分按模板用于数据。挖用闲置 args 只会追回操作容量，不再减少本方案的固定 World 尺寸；待真实模板受到容量限制后再独立评估。

## 5. 模块接口与字节码接缝

关键接口采用以下名称；辅助私有函数由各任务在所属文件内实现：

```rust
XformPrograms::try_new(Vec<XformProgramInit>) -> Result<Self, XformProgramError>
XformPrograms::empty() -> Self
XformPrograms::program_id(raw: u16) -> Option<XformProgramId>

WorldBody::bind_xform(
    &self, programs: &XformPrograms, program: XformProgramId,
    args: &[i32], owner: XformEvalOwner, tables: &WorldTables,
) -> Result<XformBinding, XformEvalError>

WorldBody::create_bullet_with_bound_xform(
    &mut self, init: BulletInit, binding: &XformBinding,
    programs: &XformPrograms, tables: &WorldTables,
) -> BulletHandle

WorldBody::run_transforms(&mut self, programs: &XformPrograms, tables: &WorldTables)
EclImage::xforms(&self) -> &XformPrograms
ImageBuilder::set_xform_programs(&mut self, programs: XformPrograms)
```

`XformEvalOwner` 是临时的 core 数据：Stage/Enemy/Bullet 三种来源，带现有实体句柄及调用任务 born_frame。出生求值另使用最终 `BulletInit` 的只读视图；效果求值使用当前有效弹。它不保存 ECL Task 引用，也不预先计算表达式没有读取的极坐标或距离。

`XformCaptureView<'a>` 也是临时只读视图：`Values(&'a [i32])` 用于绑定/出生工作区，`Segment { slots: &'a [XformSlot], data_start: u8, count: u8 }` 用于段尾持久数据。效果表达式的捕获下标在编译期重映射为紧凑持久下标，读取第 k 字对应 `slots[data_start + k/2].args[k%2]`。该视图不写入 World，不将非连续的 args 强转成连续 i32 切片，也不在每次求值前复制八字。

原 `create_bullet_with_xform`、Rust 批量创建接口、syscall 200/201/650 的签名和静态语义保留。增加两个编译器内部入口，不增加表层函数名：

| 编号 | 名称 | 正序栈参数 |
|---|---|---|
| 202 | `SYS_CREATE_BULLET_PROGRAM` | appearance, x, y, speed, angle, program_id, args_off, argc, task_script |
| 653 | `SYS_SH_XFORM_PROGRAM` | shooter_id, program_id, args_off, argc |

args_off/argc 指向当前 Task.locals 中立即消费的形参暂存区。653 绑定成功后将值复制进对应发射器，不能留下该区间的引用。202 当场绑定并建弹。编译器按引用的模板种类选择旧入口或新入口；不复用负 cnt 等隐式哨兵。

`ImageParts` 增加 `xforms: XformPrograms`，原构造处显式传空表。`EclImage.content_hash` 目前是表 coherence 哈希，不是真实脚本哈希；沿用现有身份检查，不将本轮描述为补齐脚本身份保证，不顺带扩修已记录的身份缺口。

## 6. 运行算法与失败处置

绑定：检查模板和形参数量，按值写形参；在调用者上下文计算 Bind 缓存；全部成功后才替换发射器绑定。新绑定清空 legacy off/cnt；旧 650 或 reset 清空程序绑定，避免两种来源同时生效。

出生：复制绑定到临时工作值，从同一个初始 BulletInit 计算全部 Spawn 结果；将可直接使用的值写入操作 raw 参数，并按共享映射把 C 个必要原值写入尾部数据槽。填满 16 槽、补零所有其余区域，数据槽头固定为 END；对可确定的参数做对应校验，再沿现有先段后弹路径分配。段内只写 4 B 元数据，不保留整份 XformBinding。不得让一条出生效果影响另一项出生采样。静态创建/段复用也须清掉旧 program_state 和尾槽残留。

效果进入：静态段直接取旧槽；程序段只对该槽的 Effect 参数求值，必要捕获通过段尾视图读取。先收集全部结果并完成需要的校验，再写回当前操作的 raw 参数；不改数据槽、不重新物化 LOOP 运行计数。`sampled_slot_plus_one` 防止 WAIT_SIGNAL 停驻期间重复求值；正常步进、LOOP 回跳或终止时清零。STEP tick 读取运行槽的本次参数，原始表达式仍留在共享模板。BOUNCE 等持续效果的运行参数不能在操作执行后被回收为缓存。

外观动态参数沿用 `lang::atlas` 对应的规则：set_sprite 检查形、色及组合有效性；部分设只检查对应轴，保持已有不检查组合空格的约定。编译期能确定的仍编译期拒绝；其余在绑定/出生/效果的首个可确定时点检查。stride 来自绑定表，World 不查 ECL 表。

表达式使用 ECL 现有 wrapping 整数运算、宽定点乘除和 CORDIC/查表数学。除零、非法 global 下标、非法动态 easing/通道等不 panic：绑定失败保持原发射器配置；出生失败不产生新弹或段；效果求值失败计一次 contract_viol、设置 BAD_ARGS 并终止该弹序列，弹按最后有效运动状态继续。已存在的静态安全降级（例如 bounce 次数钳位）不借此改写。

所有新增持久字段进入 checksum、SaveBytes、copy_into 和复用初始化。ENGINE_VER 由 27 升到 **28**，旧存档/回放按现有版本检查拒绝；同一镜像下复制、读档、重演必须恢复采样值及等待状态。新增零字段会改变校验和，不能要求旧/新 golden 字节相同。

## 7. 验收与范围

核心行为用最合适的一层验证：表达式数学/状态读口在 core；阶段采样、STEP、WAIT_SIGNAL、LOOP、数据区不被执行在 core 运行器；绑定隔离、阶段依赖集合及操作/数据容量诊断在 compiler 集成测试；保存/恢复扩展已有 step 存档测试。测试调用生产路径，不检查源码字符串，不复制算法，不搭额外浏览器或独立测试框架。

性能分别量：旧静态路径的改前/改后；相同轨迹、相同操作频率下的常量/直接变量/组合表达式；创建成本与持续 STEP 的稳态成本。使用 release，报告实际耗时和差值，不预设没有依据的性能百分比门槛。

本轮不改变 CI 工作流，不运行浏览器交互，不重建 Godot/Python wheel，不自动提交、推送或合并。2026-11-03 用户已在当前会话授权按本稿完善计划并实施，实施记录与实际验证结果见计划执行记录。
