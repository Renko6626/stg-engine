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
// 结构（全 12 行中位色相实测）：灰 → 五组**暗/亮对**（红·粉·蓝·青·绿，同色相、
// 后者更亮，色相差 ≤3°）→ 四级暖色梯度（黄绿 91° → 65° → 黄 60° → 橙 40°，是色相
// 差不是明暗对）→ 白。名字取标准 CSS/X11 色名。
//
// 已知特例：kunai 行（第 5 行）的第 15 格带红味，不是白——单格特例，不影响色名口径。
// 注：本图集色序与 Danmakufu 默认弹片（RED=1/ORANGE=2/YELLOW=3… 彩虹递增）**不同**，
// 那套社区常量不能按索引套过来。改名只需改本文件。
const COLOR_GRAY: int = 0;            // (164,164,164) 无彩
const COLOR_DARK_RED: int = 1;        // 0°   暗
const COLOR_RED: int = 2;             // 0°   亮
const COLOR_DARK_PINK: int = 3;       // 299° 暗
const COLOR_PINK: int = 4;            // 300° 亮
const COLOR_DARK_BLUE: int = 5;       // 240° 暗
const COLOR_BLUE: int = 6;            // 238° 亮
const COLOR_DARK_CYAN: int = 7;       // 186° 暗
const COLOR_CYAN: int = 8;            // 185° 亮
const COLOR_DARK_GREEN: int = 9;      // 141° 暗
const COLOR_GREEN: int = 10;          // 144° 亮
const COLOR_LIME: int = 11;           //  91°
const COLOR_YELLOW_GREEN: int = 12;   //  65°
const COLOR_YELLOW: int = 13;         //  60°
const COLOR_ORANGE: int = 14;         //  40°
const COLOR_WHITE: int = 15;
