# stg-ecl-compiler

离线 / 加载期 ECL 字节码编译器——把 `.ecl` 源码编译成 `EclImage`（字节码 + 常量表），
供 `stg-core` 的 VM 只读消费。只在启动时编译一次，绝不进任何热路径。

> **完整语法手册**：[`docs/ecl-lang.md`](../../docs/ecl-lang.md)（`.ecl` 脚本作者第一
> 入口——类型矩阵/语句/内建函数签名/`$` 引擎变量/xformdef/错误格式/已知限制全量参考）。
> **字节码层参考**（op/syscall/fault 码，VM/编译器开发用）：
> [`docs/ecl-ops.md`](../../docs/ecl-ops.md)。
>
> 这份 README 只是速查入口，不是权威——权威来源是上面两份文档 + 本 crate 源码
> （`src/lang/builtins.rs` 内建表、`src/lang/xform_map.rs` xform 操作表）。有出入以
> 源码 + `docs/ecl-lang.md` 为准。

## 一分钟样例

```ecl
const RANK_SLOT: int = 0;

xformdef WIND_CHIME { set_speed(2.0fx); @30 turn(90deg); }

async sub patrol() {
    loop {
        move_to(180, -120fx, 100fx, 2); wait(180);
        move_to(180,  120fx, 100fx, 2); wait(180);
    }
}

async sub timer_ui(spell: int) {
    var t: int = 600;
    while t > 0 {
        boss_set(0, $self_hp as fx / $self_hp_max as fx, spell, t, 1, 1);
        wait(60);
        t = t - 60;
    }
}

sub main() {
    spawn patrol();
    spawn timer_ui(1);
    var base: angle = 0deg;
    loop {
        var ways: int = 28 + global(RANK_SLOT) * 2;
        for i in 0..5 {
            _ = batch(i % 4, $self_x, $self_y, ways, base, 0deg, 1,
                      1.0fx + i as fx * 0.25fx, 0fx);
        }
        base = base + 7deg;
        wait(50);
    }
}
```

## 关键字与语法速查

- **类型**（三型，无隐式转换）：`int`（裸整数）· `fx`（`1.5fx`/`1.5px`，Q16.16）·
  `angle`（`90deg`/`16384bam`，BAM）。cast 白名单：`int as fx` / `fx as int`
  （**向零截断**）/ `int as angle` / `angle as int`；其余组合编译错误。
- **声明**：`sub name(p: ty, ...) { ... }` · `async sub`（只能 `spawn`，不能同步调用，
  开新协程次帧首跑）· `const name: ty = expr;`（编译期折叠）·
  `xformdef NAME { op(args); @wait op(args); ... }`（弹变换序列）。
- **语句**：`var name: type = expr;` · 赋值 · `if c {} else {}` · `while c {}` ·
  `loop {}` · `for i in a..b {}`（半开区间）· `break` / `continue` · `wait(n);` ·
  `spawn f(args);` · `return;` · 表达式语句（**有返回值必须消费**，不用就
  `_ = fire(...);` 显式丢弃）。
- **`$` 引擎变量**（只读，读取即 syscall）：`$frame` `$player_x`/`$player_y`
  `$self_x`/`$self_y` `$self_hp`/`$self_hp_max` `$self_age`。
- **内建函数**（签名以 `src/lang/builtins.rs` 为准）：`fire` `batch` `spawn_enemy`
  `drop_item` `move_to` `boss_set` `pulse_signal` `rand` `global`/`set_global`
  `aim_player` `sin`/`cos`，以及弹 setter 族——`set_speed` `set_angle` `turn` `set_vel`
  `set_ang_vel` `set_accel` `set_gravity` `stop_fx` `aim_at_player`。
- **xformdef 操作名**：见 [`docs/xform-ops.md`](../../docs/xform-ops.md)（`turn`/
  `set_speed`/`step_speed`/`step_angle` 等；STEP 族物理占 2 槽，编译器自动补 scratch）。

类型矩阵的合法/非法组合表、`sub`/`async sub` 的实参槽位硬约束、错误格式契约等完整细节
一律在 [`docs/ecl-lang.md`](../../docs/ecl-lang.md)，不在这里重复。

## 编译流水线（crate 内部结构）

```
.ecl 源码 → lex → parse → entryck → typeck → slots → codegen → EclImage
                                    └─ (若 DebugInfo::Full) → EclDebugSymbols
```

`src/lang/` 下每个阶段一个模块，模块文档写了各自的契约（判型规则、槽分配算法、
codegen 降低模板等）。`src/lib.rs` 的 `ImageBuilder`/`SubBuilder` 是 M1 阶段遗留、
现已降级为 codegen 后端的 builder DSL，不是脚本作者应该直接使用的接口。

## 编译产出

`compile(src, file)` 返回纯运行时镜像 `EclImage`（字节码 + 入口表 + 内容哈希）。
调试信息**永不出现在运行时镜像中**——`EclImage` 在有无调试信息时逐字节完全相同。

`compile_with_options(src, file, CompileOptions { debug_info: DebugInfo::Full },
stg_core::consts::ENGINE_CONSTS, Some(&stg_core::tables::TABLES_V0))`
返回 `CompiledEcl { image, debug: Some(EclDebugSymbols) }`——结果是两件分离的产物
（第四参 `engine_consts` 是脚本可见引擎常量注入表，第五参 `table: Option<&WorldTables>`
是颜色轴刀 T3 加的表绑定——`Some(t)` 时把 `t.content_hash` 盖进产出镜像并注入表派生常量
`BULLET_COLOR_STRIDE`，`None` 时不绑表、也不注入它；`compile`/`compile_for_table` 是
围绕它的两层薄封装，见 [`docs/ecl-lang.md`](../../docs/ecl-lang.md)"引擎常量"节）：
- `.image`：与 `compile` 输出逐字节相同的运行时镜像（可传入 `step` / `step_with_director`）。
- `.debug`：可选的调试符号侧载（`EclDebugSymbols`），包含 sub 名称/参数名/PC 区间/
  源码定位（文件/行/列）——**不参与确定性计算**，仅用于开发期诊断、反汇编、PC → 源码映射。`EclImage` 本身不会因调试信息的开启或关闭产生任何字节差异，这一属性在编译期测试中被断言押运。
