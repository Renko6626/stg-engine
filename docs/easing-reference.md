# 缓动语义、引擎编码与原作编号

2026-10-23 核查。以下三张表分别保存我方编码、TH06/NC 指令语义、TH18 mode 语义。
**跨作品转换只选择语义名称，不把源编号直接转换成另一个数字。** 用户已确认先补齐曲线，
将三种状态型操作单独处理。当前引擎支持30条曲线，`ENGINE_VER = 25`。

## 统一约定

- `t` 表示已完成的时间比例，`0 ≤ t ≤ 1`。通常用 `起点 + (终点−起点) × f(t)` 插值。
- `IN` 是加速起步；`OUT` 是减速结束；`IN_OUT` 是先加速后减速；`OUT_IN` 是先减速后加速。
- 二次、三次、四次分别使用 `t²`、`t³`、`t⁴`。名称不能用“快／慢”代替曲线定义。
- 表中公式描述数学形状。原生 f32 与我方 Q16.16／查表之间仍有量化差异，不承诺逐位保真。
- 内部编号仅供引擎实现与镜像编码使用；ECL 作者与导入器使用 `EASE_*` 名称。
- 原作编号要带作品、版本和指令上下文。TH06 opcode、TH06 内部缓动值、TH18 mode 是三种不同的编号。

定义两个组合曲线，避免重复写分段公式：

```text
IN_OUT(n,t) = 0.5 × (2t)^n                    当 t < 0.5
              1 − 0.5 × (2−2t)^n             当 t ≥ 0.5

OUT_IN(n,t) = 0.5 × [1 − (1−2t)^n]            当 t < 0.5
              0.5 + 0.5 × (2t−1)^n           当 t ≥ 0.5

BACK(a,t)   = (t² − 2at) / (1−2a)
```

`BACK(a,t)` 在 `t=a` 到达反向极小值，然后前往终点；它的反向版本为 `1−BACK(a,1−t)`。
名称中的 `P25/P30/P35/P38/P40` 表示IN基准曲线的极值时刻占时长的百分比，
对应OUT镜像曲线的极值位于 `t=1−a`；这些百分比不是原作编号。

## 表一：我方名称 → 功能 → 内部编号

当前事实以 [Rust Easing 枚举](../crates/stg-core/src/math/easing.rs) 为准。
[常量注册表](../crates/stg-core/src/consts.rs) 直接引用枚举值，将名称注入 ECL，不另抄编号。

| ECL 名称 | 功能 `f(t)` | 内部编号 |
|---|---|---:|
| `EASE_LINEAR` | 匀速，`t` | 0 |
| `EASE_QUAD_IN` | 二次加速，`t²` | 1 |
| `EASE_QUAD_OUT` | 二次减速，`1−(1−t)²` | 2 |
| `EASE_QUAD_IN_OUT` | 二次先加速后减速，`IN_OUT(2,t)` | 3 |
| `EASE_CUBIC_IN` | 三次加速，`t³` | 4 |
| `EASE_CUBIC_OUT` | 三次减速，`1−(1−t)³` | 5 |
| `EASE_CUBIC_IN_OUT` | 三次先加速后减速，`IN_OUT(3,t)` | 6 |
| `EASE_SMOOTHSTEP` | 平滑起停，`3t²−2t³` | 7 |
| `EASE_QUART_IN` | 四次加速，`t⁴` | 8 |
| `EASE_QUART_OUT` | 四次减速，`1−(1−t)⁴` | 9 |
| `EASE_QUART_IN_OUT` | 四次先加速后减速，`IN_OUT(4,t)` | 10 |
| `EASE_QUAD_OUT_IN` | 二次先减速后加速，`OUT_IN(2,t)` | 11 |
| `EASE_CUBIC_OUT_IN` | 三次先减速后加速，`OUT_IN(3,t)` | 12 |
| `EASE_QUART_OUT_IN` | 四次先减速后加速，`OUT_IN(4,t)` | 13 |
| `EASE_HOLD` | 在插值期间保持起点，`f(t)=0`；到时由插值器写精确终点 | 14 |
| `EASE_STEP_START` | 第一次推进时到终点，`f(t)=1`；不是在发出指令时同步完成 | 15 |
| `EASE_SINE_IN` | 正弦加速，`1−cos(πt/2)` | 16 |
| `EASE_SINE_OUT` | 正弦减速，`sin(πt/2)` | 17 |
| `EASE_SINE_IN_OUT` | 正弦先加速后减速，`(1−cos(πt))/2` | 18 |
| `EASE_SINE_OUT_IN` | 前半段 `sin(πt)/2`，后半段 `1−sin(πt)/2` | 19 |
| `EASE_BACK_IN_P25` | 先反向再前进，`BACK(0.25,t)` | 20 |
| `EASE_BACK_IN_P30` | 先反向再前进，`BACK(0.30,t)` | 21 |
| `EASE_BACK_IN_P35` | 先反向再前进，`BACK(0.35,t)` | 22 |
| `EASE_BACK_IN_P38` | 先反向再前进，`BACK(0.38,t)` | 23 |
| `EASE_BACK_IN_P40` | 先反向再前进，`BACK(0.40,t)` | 24 |
| `EASE_BACK_OUT_P25` | 超过终点再回到终点，`1−BACK(0.25,1−t)` | 25 |
| `EASE_BACK_OUT_P30` | 超过终点再回到终点，`1−BACK(0.30,1−t)` | 26 |
| `EASE_BACK_OUT_P35` | 超过终点再回到终点，`1−BACK(0.35,1−t)` | 27 |
| `EASE_BACK_OUT_P38` | 超过终点再回到终点，`1−BACK(0.38,1−t)` | 28 |
| `EASE_BACK_OUT_P40` | 超过终点再回到终点，`1−BACK(0.40,1−t)` | 29 |

`HOLD` 和 `STEP_START` 的开始／结束边界应通过实际插值器验证，不能只检查纯函数端点。
超调曲线允许 `f(t)<0` 或 `f(t)>1`，不能把输出钳制到 `[0,1]`。

## 表二：TH06 / TH06NC 编号 → 功能 → 我方语义名称

TH06 不是把通用 mode 参数传给这些移动指令，而是由 **opcode 本身**选择曲线。
“内部值”列是 `movementEaseType`，用于核对消费者，不能拿它当源 ECL 的 mode 参数。

| 源 opcode（十进制） | 指令组 | 功能 | 源内部值 | 我方语义名称 |
|---|---|---|---:|---|
| 56 | `move_position_time_linear` | 匀速，`t` | 0 | `EASE_LINEAR` |
| 52 / 57 / 61 | `move_dir_time_decelerate` / `move_position_time_decelerate` / `move_time_decelerate` | 二次减速，`1−(1−t)²` | 1 | `EASE_QUAD_OUT` |
| 53 / 58 / 62 | 对应的 `decelerate_fast` | 四次减速，`1−(1−t)⁴` | 2 | `EASE_QUART_OUT` |
| 54 / 59 / 63 | 对应的 `accelerate` | 二次加速，`t²` | 3 | `EASE_QUAD_IN` |
| 55 / 60 / 64 | 对应的 `accelerate_fast` | 四次加速，`t⁴` | 4 | `EASE_QUART_IN` |

原生实现以剩余时间比例 `r=1−t` 计算。例如内部值1使用 `1−r²`；内部值4使用 `(1−r)⁴`。
不能把 `r` 当成已完成比例，进而把加速和减速认反。

TH06NC 已核对指令选择分支 `0x1400272b1–0x1400273b3` 和曲线消费分支
`0x140028840–0x1400288f6`，与TH06上述五条曲线相符。TH06证据为
`EclManager.hpp` 的opcode枚举，以及 `EclManager.cpp` 的移动指令分派和
`movementEaseType` switch。社区反编译源：[GensokyoClub/th06](https://github.com/GensokyoClub/th06/blob/master/src/EclManager.cpp)。

历史TH06转写把四次曲线近似为三次曲线。当前已支持四次曲线，新转写使用准确名称；
既有训练卡池没有自动迁移，旧卡的近似事实仍须保留，不能仅替换文档就当作修复轨迹。

## 表三：TH18 v1.00a mode → 功能 → 我方语义名称

已静态核对 `0x402fb0` 的归一化函数及 `0x4035d8` 分支表，并核对标量插值器
`0x41f640`、运动插值器 `0x439a00` 的特殊分支。下面覆盖0–31，包含素材没有实际用到的模式；
这是指令能力调查，不是32种模式均已进行原游戏实机验证。

| 源 mode | 功能 | 我方语义名称／处理方式 |
|---:|---|---|
| 0 | 匀速，`t` | `EASE_LINEAR` |
| 1 | 二次加速，`t²` | `EASE_QUAD_IN` |
| 2 | 三次加速，`t³` | `EASE_CUBIC_IN` |
| 3 | 四次加速，`t⁴` | `EASE_QUART_IN` |
| 4 | 二次减速，`1−(1−t)²` | `EASE_QUAD_OUT` |
| 5 | 三次减速，`1−(1−t)³` | `EASE_CUBIC_OUT` |
| 6 | 四次减速，`1−(1−t)⁴` | `EASE_QUART_OUT` |
| 7 | 每帧 `value += rate` | 状态型累加，不能映射为 `EASE_LINEAR` |
| 8 | 三次Hermite插值，使用两个端点和两个切线参数 | 额外参数型插值，不能映射为 `EASE_CUBIC_*` |
| 9 | 二次先加速后减速，`IN_OUT(2,t)` | `EASE_QUAD_IN_OUT` |
| 10 | 三次先加速后减速，`IN_OUT(3,t)` | `EASE_CUBIC_IN_OUT` |
| 11 | 四次先加速后减速，`IN_OUT(4,t)` | `EASE_QUART_IN_OUT` |
| 12 | 二次先减速后加速，`OUT_IN(2,t)` | `EASE_QUAD_OUT_IN` |
| 13 | 三次先减速后加速，`OUT_IN(3,t)` | `EASE_CUBIC_OUT_IN` |
| 14 | 四次先减速后加速，`OUT_IN(4,t)` | `EASE_QUART_OUT_IN` |
| 15 | 归一化函数返回0，保持起点，到时写终点 | `EASE_HOLD` |
| 16 | 归一化函数返回1，首次推进即取终点 | `EASE_STEP_START` |
| 17 | 每帧先 `value += velocity`，再 `velocity += acceleration` | 状态型累加，不能映射为固定加速曲线 |
| 18 | `sin(πt/2)`，正弦减速 | `EASE_SINE_OUT` |
| 19 | `1−cos(πt/2)`，正弦加速 | `EASE_SINE_IN` |
| 20 | 正弦先减速后加速 | `EASE_SINE_OUT_IN` |
| 21 | 正弦先加速后减速 | `EASE_SINE_IN_OUT` |
| 22 | `BACK(0.25,t)` | `EASE_BACK_IN_P25` |
| 23 | `BACK(0.30,t)` | `EASE_BACK_IN_P30` |
| 24 | `BACK(0.35,t)` | `EASE_BACK_IN_P35` |
| 25 | `BACK(0.38,t)` | `EASE_BACK_IN_P38` |
| 26 | `BACK(0.40,t)` | `EASE_BACK_IN_P40` |
| 27 | `1−BACK(0.25,1−t)` | `EASE_BACK_OUT_P25` |
| 28 | `1−BACK(0.30,1−t)` | `EASE_BACK_OUT_P30` |
| 29 | `1−BACK(0.35,1−t)` | `EASE_BACK_OUT_P35` |
| 30 | `1−BACK(0.38,1−t)` | `EASE_BACK_OUT_P38` |
| 31 | `1−BACK(0.40,1−t)` | `EASE_BACK_OUT_P40` |

注意：18是OUT、19是IN；20是OUT_IN、21是IN_OUT，不能按惯常库的排列猜编号。
22–31的原生常量是f32，表中 `BACK` 是归一化数学描述，不声称与原生运算逐位相同。
特殊mode7/8/17在纯函数内虽然落到线性分支，但实际插值器在调用纯函数前已单独处理，
只看纯函数分派表会误判。

## 转换器如何使用这些表

转换方向为：

```text
TH06 opcode / TH18 mode
        ↓ 按对应作品的语义表识别
我方语义名称，例如 EASE_QUAD_OUT
        ↓ 输出可人工校对的 .ecl
现有 ECL 编译器查注册表，编码成引擎内部值
```

例如TH18的 `movePosTime(..., mode=4, ...)` 输出
`move_to(..., EASE_QUAD_OUT)`；TH06的57号指令也选择 `EASE_QUAD_OUT`。
导入器不用知道该名称在引擎里编码为多少。

我方枚举／常量注册表归引擎维护；两个作品的源编号识别归各自导入器维护，不把作品方言表
放入确定性数学内核。未知mode与未实现目标须显式报错，不默认换成匀速或相近曲线。

## 实现范围与兼容性

- 现有0–7编码与烘焙表字节保持不变，追加8–29。每条257个i32采样，新增22,616字节。
- 运动写口和弹STEP的创建校验统一使用 `Easing::from_id`，无效编号仍按P4拒绝并计数。
- 烘焙表保留负值和超1值；运动和STEP使用宽中间量插值，终帧仍写精确目标。
- HOLD和STEP_START不改变原有启动时机；敌人运动在首次积分推进，弹STEP发射当帧只初始化、次帧才推进。
- ENGINE_VER24→25：此前无效的8–29成为合法输入，版本身份随之更新；World布局未变。
- TH18 mode7/8/17不属于本曲线集合；ENGINE_VER26已提供独立accum／accel／Hermite值槽，显式读取。
  原生状态、参数和指令注入方式见[状态型插值核查](stateful-interpolation-reference.md)。
- 尚未重建训练wheel、升级训练依赖或迁移既有训练卡池。正式使用新增曲线前，必须同步本地依赖、
  锁文件、Magnus wheel和SHA256SUMS。未启动训练或原游戏实机验收。
