---
name: writing-danmaku-ecl
description: Use when writing, editing, or debugging `.ecl` danmaku scripts for the stg-engine repo — bullet patterns, enemy movement, spell cards, shooters, xformdef sequences. Also use when a `.ecl` compiles but the pattern on screen is wrong, when bullets vanish or never appear, or when tasks stop running with no error.
---

# 给 stg-engine 写 .ecl 弹幕

`.ecl` 是这个引擎的弹幕脚本语言。**语法是 Rust 味的，门槛不在语法**——在三型无隐式转换、
在协程模型、在几处「编译得过但跑出来不对」的跨机制交互。

这份 skill 不重复手册。它给三样东西：**验证循环**、**该读哪一篇**、
**跨机制的静默坑**（那些不在任何单篇文档里、只能靠逐帧二分才找得到的）。

## 验证循环（写一段验一段，别写完一屏再调）

```bash
cargo run -p stg-harness -- check <file.ecl|目录>   # 语法：行列报错 + 源行 + ^ 定位
cargo run -p stg-harness -- run   <file.ecl|目录>   # 跑起来：弹/敌/任务/fault/峰值
cargo run -p stg-harness -- run   <f.ecl> --at 45   # 第 45 帧每颗弹的角度/速度/sprite
cargo run -p stg-harness -- serve --ecl <f.ecl>     # 浏览器里真看（ssh -L 转发）
```

`run` 的退出码：**有 task fault 就非零**。`--rank 0..3` 试难度分档，`--frames N` 控时长。

**`--at F` 是判断"对不对"的主力**，不是可选项。弹数对不上只能说明数量，`--at` 才看得出
**环闭没闭合、是不是 N 路均分、速度层对不对、角度是不是差了 90°**。改完弹幕不看 `--at`，
等于没验。

跑真 demo 局：`run godot/ecl/game --frames 6000`（目录整取编译）。

## 该读哪一篇

`docs/ecl-lang.md` 是薄索引，正文在 `docs/ecl-lang/`：

| 要干什么 | 读 |
|---|---|
| 第一次写，从零到一个弹幕 | `1-hello-danmaku.md` |
| 任务/协程、`wait`、`spawn`、敌退场 | `2-tasks.md` |
| 造敌、运动动词、死亡与掉落 | `3-enemy.md` |
| 发弹：发射器 / `fire` / `batch` / xformdef | `4-bullets.md` |
| 三型、后缀、运算矩阵、cast | `5-types.md` |
| 符卡、整局编排、多文件 | `6-spell-and-stage.md` |
| 内建签名、`$` 引擎变量、引擎常量 | `7-reference.md` |
| 什么 Fault、什么静默降级 | `8-errors.md` |

`docs/ecl-ops.md` 是字节码层（syscall 号表），**写弹幕通常不需要**。
`godot/ecl/game/` 与 `crates/stg-harness/scenes/rainbow.ecl` 是可抄的真实内容。

## 跨机制的静默坑

这些**编译得过、不报错、不计数**，只有逐帧看才发现。它们不在任何单篇文档里，
因为每一条都横跨两个机制。

### `@N op()` 是后置延迟，不是时间标签

```text
xformdef X { set_speed(2.0fx); @30 turn(90deg); }   // ✗ 两条同帧跑完，弹一出生就转
xformdef X { @30 set_speed(2.0fx); turn(90deg); }   // ✓ 设速 → 飞 30 帧 → 转
```

**规则一句话：`@N` 写在哪条 op 前面，N 就是那条 op 跑完之后的等待。**

别记成"挂在前一条 op 上"——那是同一个错误的另一种说法，会让你写回 `A; @N B;`。
`@N` 永远绑定它**紧跟着的**那条 op，只是那个 N 在**该 op 执行之后**才开始数。

```text
@60 A();  B();     ⇒  第 0 帧跑 A，等 60 帧，第 60 帧跑 B
A();  @60 B();     ⇒  第 0 帧 A 和 B 一起跑完，然后空等 60 帧
```

机制：`@N` 存进**本条 slot** 的 wait 字段，而变换相**先 fire 再设 wait**。所以一段
xformdef 里**所有 `wait=0` 的 op 在同一帧连跑完**，撞上第一个带 `@N` 的 op——**那条也跑**，
跑完才停 N 帧。

demo 的 `WIND_CHIME` 曾因此静默错了很久：`@30 turn` 什么都没延迟（turn 后面已无内容）。

### xform 的 `@N` 与 `sh_task` 的 `wait(N)` 差一帧

**新任务出生当帧不跑**（次帧首跑门禁），而**弹的 xform 段出生当帧就走**。所以同样写 N，
弹任务比 xform 晚一帧。拿 xformdef 自杀（`set_life`）配 `sh_task` 里的 `sh_fire` 做分裂弹时，
母弹会**先死一帧**，"owner 死 → 静默杀任务"的门吃掉子弹，**什么都不发生、不报错**。
让任务侧比 xform 侧早一帧。

### `sh_task` 一句吃掉 N 个任务槽

`sh_count(0, 255, 1)` + `sh_task` = 一次 `sh_fire` 派 **255** 个任务，池共 **256**。
池满的降级是**弹保留、任务丢**，静默。挂 `sh_task` 前先算这一发要几个槽。

### 发射器槽是每任务 4 个

`sh_reset(0..3)`，`SHOOTERS_PER_TASK = 4`。不同任务的 0 号槽互不相干。
`sh_reset` 之前的残留会继承——每个新槽先 `sh_reset`。

### `sh_aim` 与 `aim_player()` 的基点不同

`sh_aim` 从**出弹口**（owner + `sh_offset`）瞄，`aim_player()` 从 **owner 中心**瞄。
设了 `sh_offset` 时两者不等价，越近差得越明显。两者都在**开火那一刻**解析，不是配置时。

### `batch` 不替你均分整周

`angle_step` 是逐弹增量。手算 `65536 / n` 在 n 不整除时**环合不拢**（demo 曾因此在
Easy/Normal/Lunatic 三档各差 16~18 BAM，只有 Hard 恰好整除）。**要整周环用 `sh_ring`**，
它逐颗算 `(i×65536)/n`、余数均摊、精确闭合。

### 敌主任务 `return` = 这只敌退场

不是"任务没了敌还在"。要它留着就 `loop { wait(1); }` 挂住。

## 类型：三型无隐式转换

裸整数 = `int`；`1.5fx` = `fx`（Q16.16）；`90deg` = `angle`（BAM）。
裸 `1.5` 是**词法错误**，裸 `90` 放角度位报"期待 Angle 实际 Int"。
`int as angle` 是**位穿透**不是转度数（`90 as angle` ≠ 90°）。完整矩阵见 `5-types.md`。

## 常见错法

| 症状 | 多半是 |
|---|---|
| 弹幕能编译，跑起来什么都没发生 | 任务被 fault 杀了 —— `run` 看 `task_faults`，非零会打出码与原因 |
| 挂了 `sh_task` 的弹一个任务都没跑 | 任务池满（一发 N 颗 = N 个槽），或母弹先死一帧 |
| 环差一点点合不拢 | 用了 `batch` 手算 `angle_step` —— 换 `sh_ring` |
| xformdef 的延迟不生效 | `@N` 挂错了 op（它是后置延迟） |
| 敌人放出来立刻消失 | 主任务 `return` 了 |
| 角度整体偏 90° 或镜像 | `atan2(y, x)` 的 y 在前；`int as angle` 是位穿透 |

## 收口

内容改动不碰 `crates/`。改完至少：

```bash
cargo run -p stg-harness -- check godot/ecl/game     # 整局编译
cargo run -p stg-harness -- run   godot/ecl/game --frames 6000
```

`run` 退非零 = 有 fault，必须查清再说"好了"。要看画面跑
`cargo build -p stg-godot && godot --path godot`（异机先读 `godot/README.md`）。
