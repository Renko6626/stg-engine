// demo 内容包的弹型/颜色词表——**引擎不注册这些名字**（mod 作者用自己的一份，
// 地位对等；见 spec §5）。值 = 形号 × BULLET_COLOR_STRIDE / 色序号。
// 图集布局见 docs/render-contract.md §3；空格见下方注释。

const BULLET_RICE: int = 0;        // 米弹   r=3
const BULLET_BALL_S: int = 16;     // 小玉   r=3
const BULLET_BALL_M: int = 32;     // 中玉   r=4
const BULLET_BALL_L: int = 48;     // 大玉   r=6
const BULLET_SCALE: int = 64;      // 鳞弹   r=4
const BULLET_KUNAI: int = 80;      // 苦无   r=4
const BULLET_SHARD: int = 96;      // 碎片   r=3
const BULLET_AMULET: int = 112;    // 札     r=5
const BULLET_STAR: int = 128;      // 星弹   r=8
const BULLET_HEART: int = 144;     // 心弹   r=6  ← 只做了低 12 色,高 4 色是空格
const BULLET_BUTTERFLY: int = 160; // 蝶弹   r=6  ← 同上
const BULLET_DROP: int = 176;      // 水滴   r=4

const COLOR_RED: int = 0;
const COLOR_ORANGE: int = 1;
const COLOR_YELLOW: int = 2;
const COLOR_CHARTREUSE: int = 3;
const COLOR_GREEN: int = 4;
const COLOR_SPRING: int = 5;
const COLOR_CYAN: int = 6;
const COLOR_AZURE: int = 7;
const COLOR_BLUE: int = 8;
const COLOR_VIOLET: int = 9;
const COLOR_MAGENTA: int = 10;
const COLOR_ROSE: int = 11;
const COLOR_WHITE: int = 12;       // ← HEART/BUTTERFLY 从这里开始是空格
const COLOR_GRAY: int = 13;
const COLOR_BLACK: int = 14;
const COLOR_GOLD: int = 15;
