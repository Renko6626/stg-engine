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
// 逐列实测（色相/明度）后的实际结构，**不是**整齐的"暗/亮成对"：
//   真正的暗/亮对只有三组：1/2 红、7/8 青、9/10 绿（同色相、后者更亮）。
//   3/4 是**色相差**（291°/303°），明度几乎相同——所以不叫 MAGENTA_DARK/MAGENTA。
//   5/6 色相与明度同时变（249°/233°），是靛→蓝。
//   11..14 是一条色相梯度：黄绿 95° → 73° → 黄 60° → 橙 41°，不是明暗对。
//   两端 0/15 是灰与白。
// 注：本图集的色序与 Danmakufu 默认弹片（RED=1/ORANGE=2/YELLOW=3… 彩虹递增）**不同**，
// 那套常量不能按索引套过来。名字按实测色相取，改名只需改本文件。
const COLOR_GRAY: int = 0;
const COLOR_RED_DARK: int = 1;    // 0°  暗
const COLOR_RED: int = 2;         // 0°  亮
const COLOR_PURPLE: int = 3;      // 291°
const COLOR_MAGENTA: int = 4;     // 303°
const COLOR_INDIGO: int = 5;      // 249°
const COLOR_BLUE: int = 6;        // 233°
const COLOR_CYAN_DARK: int = 7;   // 182° 暗
const COLOR_CYAN: int = 8;        // 184° 亮
const COLOR_GREEN_DARK: int = 9;  // 142° 暗
const COLOR_GREEN: int = 10;      // 146° 亮
const COLOR_CHARTREUSE: int = 11; //  95°
const COLOR_YELLOW_GREEN: int = 12; // 73°
const COLOR_YELLOW: int = 13;     //  60°
const COLOR_ORANGE: int = 14;     //  41°
const COLOR_WHITE: int = 15;
