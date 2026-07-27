// demo 内容包的弹型/颜色词表——**引擎不注册这些名字**（mod 作者用自己的一份，
// 地位对等；见 spec §5）。值 = 行号 × BULLET_COLOR_STRIDE / 列号。
//
// 图集：`godot/assets/bullets.png`（16 列 × 12 行 × 16px），由
// `godot/tools/slice_bullet_sheet.gd` 从弹片切出；行序即下表顺序。
// 半径住 `crates/stg-core/src/tables.rs` 的 SHAPE_RADIUS，改那里不用改这里。
//
// 12 行全满 16 色（切图工具逐格实测），**本图集没有空格**——所以任何一行都可以
// 安全地轮转全色（`i % BULLET_COLOR_STRIDE`）。将来若补入有缺色的弹型，
// 记得在 tables.rs 的掩码里标空格，并遵守"缺色留空、不压紧"的摆位纪律（spec §5.2）。

// ── 弹型（行）────────────────────────────────────────────────────────────
// 不带 BULLET_ 前缀：本表自己就有一个叫 `bullet` 的弹型，`BULLET_BULLET` 太丑。
const LASER: int = 0;        // 激光条（竖条纹，接成一束激光用）  判定 r=3
const ARROWHEAD: int = 16;   // 箭头                              r=4
const OUTLINE: int = 32;     // 环（中空圆）                      r=4
const BALL: int = 48;        // 玉（实心圆）                      r=5
const RICE: int = 64;        // 米弹（细长粒）                    r=2
const KUNAI: int = 80;       // 苦无                              r=3
const SHARD: int = 96;       // 碎片（细菱）                      r=2
const AMULET: int = 112;     // 札                                r=5
const BULLET: int = 128;     // 弹丸（胶囊形）                    r=3
const BACTERIA: int = 144;   // 菌形（带暗心的椭圆）              r=3
const STAR: int = 160;       // 星                                r=4
const LASERHEAD: int = 176;  // 激光头（激光束端头的圆帽）        r=5

// ── 颜色（列）────────────────────────────────────────────────────────────
// 原作是「暗/亮成对」的排法：1/2 红、3/4 品红、5/6 蓝、7/8 青、9/10 绿各为一对
// （前暗后亮）；11..14 是黄绿→黄→橙的色相梯度；两端 0/15 是灰与白。
// 名字按逐列实测色相取，改名只需改本文件。
const COLOR_GRAY: int = 0;
const COLOR_RED_DARK: int = 1;
const COLOR_RED: int = 2;
const COLOR_MAGENTA_DARK: int = 3;
const COLOR_MAGENTA: int = 4;
const COLOR_BLUE_DARK: int = 5;
const COLOR_BLUE: int = 6;
const COLOR_CYAN_DARK: int = 7;
const COLOR_CYAN: int = 8;
const COLOR_GREEN_DARK: int = 9;
const COLOR_GREEN: int = 10;
const COLOR_LIME: int = 11;
const COLOR_YELLOW_GREEN: int = 12;
const COLOR_YELLOW: int = 13;
const COLOR_ORANGE: int = 14;
const COLOR_WHITE: int = 15;
