# 按作品区分的弹型目录

2026-11-05。内建内容追加经典TH06十种、TH18 v1.00a四十八种普通弹型，出生时直接取各自的基础半径。目录在ENGINE_VER31接入，WorldTables二进制格式仍为v6；ENGINE_VER32起可按局选择[碰撞规则](collision-rules.md)，贴图编号不自动选择公式。

## 名字、半径与贴图

唯一维护输入是[目录](../godot/assets/bullet/catalog.json)与两份合成图源：[TH06](../godot/assets/bullet/source_th06.png)、[TH18](../godot/assets/bullet/source_th18.png)。原作普通飞行态图块按64px格居中排放，小弹保持原来的像素尺寸。图源允许人工编辑；需要调整名字、尺寸或色槽时同步改目录。

目录的每一型记录作品、原type、常量名、逻辑行基址、Q16.16半径、有效原作色号，以及图源中的裁切矩形。基础半径按原作数据取最近可表示定点值；例如2.4为raw157286。原始exe/ANM核查证据存训练仓调查记录，不是构建依赖。

| 逻辑编号区间 | 内容 |
|---|---|
| 0–191 | 原有12种×16色，编号、半径和贴图保持 |
| 192–351 | `TH06_*`，行基址=192+原type×16 |
| 352–1119 | `TH18_*`，行基址=352+原type×16 |

全部名字在[生成词表](../godot/ecl/game/touhou-bullets.ecl)。常用对照：

| 名字 | 行基址 | 基础半径 |
|---|---:|---:|
| TH06_RICE | 224 | 2 |
| TH18_RICE | 480 | 2.4 |
| TH06_BALL | 240 | 3 |
| TH18_BALL | 416 | 4 |
| TH06_BUBBLE | 336 | 16 |
| TH18_BUBBLE | 864 | 14 |
| TH18_BIG_YIN_YANG_R | 1088 | 28 |

TH06使用经典原版贴图；NC十型的基础半径已单独核实与经典版相同，NC的高清美术和ANM动画未在此复刻。经典TH06原生盒形判定、NC半径相加、TH18平方和公式仍有区别，目录不以改半径抵消这些公式差异。

## 脚本使用

把词表文件放入当前ECL编译目录，或复制实际使用的常量；这些是内容包名字，不是全局引擎关键字。新弹型直接传原作色号，无须把8色映射到旧16色调色板：

```ecl
const TH06_RICE: int = 224;
const TH18_RICE: int = 480;
sub main() {
    _ = fire(TH06_RICE, 2, -32fx, 96fx, 2fx, 90deg, none, none);
    _ = fire(TH18_RICE, 2, 32fx, 96fx, 2fx, 90deg, none, none);
    loop { wait(60); }
}
```

同一型始终占16个逻辑槽。原作缺色的槽标为无效，直接创建会被拒绝；部分换色落到无效槽时显示透明格，沿用既有约定。TH18固定色火球/音符忽略输入色号，所以16槽均可用并共享同一首帧。TH06火弹0–3是动画初始帧选择，不能当成四种颜色。

普通弹出生半径直接写入弹池，`none`变换即可使用，不为静态半径分配xform。原有`set_sprite`、`set_shape`、`set_color`继续只改变外观，保留出生半径；本轮没有加入动态HITBOX/SIZE写口，也没有把换外观改成刷新半径。

## 编号域与表现端

弹池原字段`sprite`与RL观测`TYPE`现在保存逻辑appearance。渲染用`WorldTables::bullet_sprite(id)`得到实际图集格号；多个逻辑弹型可以共用同一图。Godot实时、影子、倒放和消弹淡出均执行这一步。harness viewer和`run --at`的`sprite`列输出的是物理图号，Python静态预览直接消费；RL回放在解码TYPE时映射一次。

[布局文件](../godot/assets/bullet/atlas-layout.json)同时提供逻辑映射和物理网格。当前图集为1024×2560，64px格、16列40行，635个物理格；透明格为192。1120个逻辑项中919项有效、201项缺色。原物理0–191保持格号，小图在64格内居中、不放大。

## 重烘焙

```bash
source scripts/find-godot.sh
"$GODOT_BIN" --headless --path godot --script res://tools/slice_bullet_sheet.gd
cargo run -p stg-harness -- bake-tables
cargo run -p stg-harness -- verify-tables
```

第一步只依赖入库的目录和图源，生成PNG、布局JSON、Rust整数追加表与ECL词表；第二步把追加表写入现有WorldTables格式。目录、图源及生成产物一并维护，不手改生成的Rust或ECL文件。core不加载PNG或浮点数。更新表会改变内容hash，harness、wheel和训练消费端须使用同一批表。

图源保留普通态静态帧；出生减速/出生无判定已按训练取舍省略，旋转、闪烁和多帧ANM未通用复现。现有曲线激光暂缓范围不变。
