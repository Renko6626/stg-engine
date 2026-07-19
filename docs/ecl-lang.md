# ECL 表层语言参考（作者第一入口）

> **这是什么**：`.ecl` 脚本作者手册——语法、类型、内建函数、`$` 引擎变量、xformdef、错误格式。
> **权威来源**（冲突时以它们为准）：`crates/stg-ecl-compiler/src/lang/`（`builtins.rs` 内建表 /
> `xform_map.rs` xform 操作表）· spec `docs/superpowers/specs/2026-07-18-m19-ecl-language.md`。
> 字节码层参考（op/syscall/fault 码）见 [`ecl-ops.md`](ecl-ops.md)——作者通常不需要看它。
> 编译时机：启动时从源码文本编译（`lang::compile`），编译器确定性有测试押运。

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

## 类型：`int / fx / angle`（三型，无隐式转换）

- 字面量：裸整数 = `int`；`1.5fx`/`1.5px` = `fx`（Q16.16，精确十进制折叠、round-half-even）；
  `90deg` = `angle`（BAM）；`16384bam` = `angle` 原值。
- 运算矩阵（非法组合 = 编译错误并提示 cast）：
  `int⊕int→int` · `fx±fx→fx` · `fx*int→fx`（普通乘）· `fx*fx→fx`（定点乘）· `fx/int→fx` ·
  `fx/fx→fx`（定点除）· **`int/fx` 非法**（方向性陷阱：整除会错位 65536 倍）·
  `angle±angle→angle`（回绕）· **`angle` 乘除一律非法** · 同型比较→`int`(0/1) ·
  `&&`/`||`/`!` 仅 `int`（短路求值）。
- cast 白名单：`int as fx`（×65536）· `fx as int`（÷65536，**向零截断**——与引擎内算术右移
  对负数不同，`-1.5fx as int == -1`）· `int as angle`/`angle as int`（位穿透）。其余组合拒绝。

## 语句

`var name: type = expr;` · 赋值 · `if c {} else {}` · `while c {}` · `loop {}` ·
`for i in a..b {}`（半开区间，`i` 为 `int`）· `break`/`continue` · `wait(n);`（n: int 帧）·
`spawn f(args);` · `return;` · 表达式语句（**值必须消费**——有返回的内建不接收就
`_ = fire(...);` 显式丢弃，不丢弃 = 编译错误；这是"忘 POP 远处爆栈"足枪的语言层灭除）。

## sub 与 async sub（调用途径强制分离）

- **`sub`**：只能被**同步调用**（`f(args);` 语句）。参数/局部变量由编译器静态分配 locals 槽
  （调用图着色）；**禁递归**（直接/间接均编译错误，报错含环路径）；sub 无返回值。
- **`async sub`**：只能被 **`spawn`**（或 `fire` 的 task 引用）——开新协程（次帧首跑），
  实参拷进新任务；**不能被同步调用**（编译错误）。
- 同一 sub 想两用？拆成两个——这是实参槽位健全性的硬约束，编译器不放行。
- 容量红线（编译期检查）：单任务 locals 总量 ≤64 字、求值栈深 ≤32、调用深 ≤8。

## `$` 引擎变量（只读；读取即 syscall）

`$frame:int` · `$player_x/$player_y:fx` · `$self_x/$self_y:fx` · `$self_hp/$self_hp_max:int` ·
`$self_age:int`（**任务**出生帧龄）。`self_*` 按任务 owner 解析（敌/弹/关卡）。

## 内建函数（签名以 `builtins.rs` 为准）

`fire(appearance:int, x:fx, y:fx, speed:fx, angle:angle, xf:XFORMDEF名|none, task:ASYNC_SUB名|none) -> int` ·
`batch(appearance, x, y, n_angle:int, angle0:angle, angle_step:angle, n_speed:int, speed0:fx, speed_step:fx) -> int` ·
`spawn_enemy(x,y,hp,drop_table,score) -> int` · `drop_item(x,y,ty) -> int` ·
`move_to(dur:int,x:fx,y:fx,easing:int)` · `boss_set(slot,ratio:fx,spell,timer,phase,active)` ·
`pulse_signal(ch)` · `rand(n:int) -> int` · `global(n) -> int` · `set_global(n,v)`
（槽 0-15 系统段脚本只读）· `aim_player() -> angle` · `sin/cos(a:angle) -> fx` · 弹 setter 族。

## xformdef（弹变换序列声明）

```ecl
xformdef NAME { op(args); @wait op(args); ... }
```

- op 名 = [`xform-ops.md`](xform-ops.md) 小写助记（`turn`/`set_speed`/`set_ang_vel`/
  `step_speed`…）；`@N` 前缀 = 该槽 wait N 帧；参数必须**编译期常量**（字面量/const/一元负号）。
- **STEP 族（`step_speed`/`step_angle`）物理占 2 槽**——scratch 由编译器自动补，作者按 1 条写；
  物理槽总数 ≤16。`loop`/`end` 不开放（复杂控制流写任务弹；尾部零填充天然 END）。
- 被 `fire(..., NAME, ...)` 引用才占 locals 空间（3 字/物理槽，算进引用它的 sub 的容量账）。

## 错误格式与已知限制

- 错误：`文件:行:列: 说明` + 源行摘录 + `^` 定位；一个错误不吞后续（恢复到语句边界）。
- 已知限制（v1）：禁递归 · locals 静态分配（同 sub 内变量名不可重名）· sub 无返回值 ·
  **无跨语言常量引用**（appearance id/globals 槽号需手写数字镜像，见 follow-ups）·
  时间标签 `+N:` 未进 v1（显式 `wait`）。
