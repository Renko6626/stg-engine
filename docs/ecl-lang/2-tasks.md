# 2 · 任务与时间

> 这一篇讲**脚本什么时候跑**：`sub` 与 `async sub` 的分工、`wait` 的准确周期、
> 新协程什么时候开始跑。
> 读之前先读 [1 · 从零到一个弹幕](1-hello-danmaku.md)——那一篇已经用过 `spawn`/`wait` 了，
> 这一篇是把它们的确切语义补上。

## sub 与 async sub（调用途径强制分离）

### 根入口 `sub main()`

整份编译产物**必须且仅有一个**零参数 `sub main()`，`async` 不可修饰它。单文件时它就在那一个
文件里；多文件时它只出现在其中一个文件里，其余文件不需要、也不能再声明一个。

### 普通 sub vs async sub

- **`sub`** 只能被同步调用（`f(args);` 语句，或通过 `CALL` op 从其他 sub 调用）。参数和局部
  变量由编译器静态分配 locals 槽（调用图着色）。**禁递归**，直接和间接都是编译错误，报错含
  环路径。sub 无返回值。
- **`async sub`** 只能被 `spawn`（或 `fire` 的 task 引用），开新协程，实参拷进新任务，不能被
  同步调用（编译错误）。
- 同一 sub 想两用？拆成两个——这是实参槽位健全性的硬约束，编译器不放行。
- 容量红线（编译期检查）：单任务 locals 总量 ≤64 字、求值栈深 ≤32、调用深 ≤8。

⚠️ **新起的协程当帧不跑。** 创建那一帧（`born_frame`）不执行任何一条指令，下一帧才首次跑
——从"存在"的角度说是 spawn 后第 2 个 step 才真正活起来。

### 持久引用使用名称

脚本之间的持久引用（`spawn` 目标、`fire` 的 task 参数、`spell_begin` 的 pattern）一律使用
sub 名称：编译器在编译期解析名称并编码为 canonical `SubId`。两条推论：

- 这些位置**只收标识符或字面量 `none`**，不是求值表达式——`fire(..., xf_or_none_expr(), ...)`
  这种"算出来的引用"通不过。
- **不存在的 sub 名称在编译期即报错**，不存在运行期"名字未找到"的分支。

<details><summary>named entry / CallOnly / main 的 singleton 保护（引擎宿主侧接口）</summary>

- 运行时 `EclImage` 无字符串表，只有 `(SubId, code_entry)` 的扁平元数据。`spawn patrol()` 在
  编译期解析 `patrol` 到其 `SubId`，存入 `SPAWN` 指令的操作数；
  `fire(RICE, COLOR_RED, $self_x, $self_y, 0fx, 0deg, WIND_CHIME, trail_task)` 里的
  `trail_task` 同理。
- 普通 sub 的形式名称（如 `helper`）只存在于调试符号侧载（`DebugInfo::Full`），运行时
  `EclImage` 不为其保留 named entry——它们是 `CallOnly` sub，只能被其他 sub 通过 `call`
  指令调用，不能 `spawn`、不能从引擎层按名解析。
- 每个 `async sub` 都是一个**公共 named entry**，其名称注册在 `EclImage` 的 entry 表中。
  引擎层（C/Rust 宿主）可通过 `image.resolve_entry("patrol")` 按名解析，然后通过
  `world.spawn_entry()` 或 `world.spawn_entry_named()` 启动——这是跨语言/跨脚本引用的
  确定性基础。
- `sub main()` 是关卡的根入口脚本，只能通过引擎的 `start_main` / `start_main_with_owner`
  API 启动。生命周期是 **singleton**：每个 `World` 实例最多成功启动一次——即使 main 任务
  自然结束或 fault，再次调用 `start_main` 也会返回 `MainAlreadyStarted`（同时触发
  `contract_viol` 计数）。这一保护确保确定性回放中 main 不会重复派发。

</details>

## `wait(n)` 的周期就是 n

**第 F 帧执行 `wait(n)` ⇒ 第 F+n 帧接着往下跑。** 所以 `loop { …; wait(n); }` 的周期恰是
**n** 帧，`wait(1)` = 每帧跑一次。你可以放心地自己记帧数：

```text
var t: int = 0;
while t < 600 { …; wait(20); t = t + 20; }   // 恰好 600 帧，30 轮
```

**`wait(0)` = 当这句不存在**（不让出，同帧继续往下执行）。这样 `wait(delay)` 在 `delay`
算出 0 时行为自然，不必调用方特判。代价是 `wait(0)` 挡不住循环：

```text
loop { wait(0); }        // 真·死循环：烧穿单任务指令预算，Fault(3) FAULT_BUDGET
```

写在 `loop` 里的 `wait(0)` 会被 `FAULT_BUDGET` 杀掉，和 `loop {}` 一个下场——确定性的响亮
失败，不是挂死也不是 UB。要"每帧跑一次"请写 `wait(1)`。

⚠️ **`n` 被截成 `u16`（取低 16 位），静默，无诊断**：任务的等待计数器是 `u16`。所以
`wait(-1)` 等 **65535** 帧（不是"立即继续"），`wait(70000)` 悄悄变成 4464，而
**`wait(65536)` 截断成 `wait(0)`** ⇒ 按上面的规则是真 no-op，写在 `loop` 里就是死循环。
一帧 1/60 秒，65535 帧 ≈ 18 分钟，正常关卡碰不到上限；要等更久就套循环
（`for i in 0..10 { wait(30000); }`），别写一个大数上去。

这个语义有测试钉死（`ecl/vm.rs` 的 `wait_truncates_to_low_16_bits`），不是 bug，不会改。

<details><summary>2026-08-01 之前 wait 的周期是 n+1（旧脚本的补偿量要去掉）</summary>

旧实现的周期是 **n+1**（`wait(1)` 是"隔一帧跑"），上面那段 `while t < 600` 循环实际走
30×21 = **630** 帧，凡是自己记帧数的脚本一律偏 1/n。这次修正 bump 了 `ENGINE_VER`
（11 → 12），旧回放/存档拒载。若你手上有为旧语义调过参的脚本，把补偿量去掉即可：当初写
`wait(19)` 凑 20 帧的，现在改回 `wait(20)`。

</details>

## 任务结束了会怎样

`spawn` 出来的伴生任务、`fire(..., task)` 挂在弹上的任务、`spell_begin` 的 `pattern` 任务，
跑到末尾就只是它自己没了，不牵连别人。

⚠️ **只有一个例外，而且是最容易踩的一脚：敌的主任务一 `return`，这只敌当场退场**（D9）。
这条规则连同它的全部后果（退场是静默的：不掉道具、不加分、不发死亡事件）整段写在
[3 · 敌人](3-enemy.md) 开头——因为例子和后果全是敌的事。写敌任务之前先读那一段。

---

**下一篇** → [3 · 敌人](3-enemy.md)：主任务退场（D9）、生成与轮询、运动、死亡与掉落、按敌号
观察。
