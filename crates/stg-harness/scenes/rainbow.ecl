// 彩虹风铃符卡（M1.9 T4 —— .ecl 表层语言重写，金向量二号 scene 2 的脚本源）
//
// 结构照 docs/superpowers/specs/2026-07-18-m19-ecl-language.md 语言草图（"语言草图 v1"
// 就是目标形状）：main 五环 loop + patrol/timer_ui 两个 async 子任务。声明序即
// EclImage 的 script id（`EclImage` 不带名字表，见 stg-core/src/ecl/image.rs 文档）——
// 本文件保留草图原始顺序 const → xformdef → patrol → timer_ui → main，故
// main 的 script id = 声明序最后一个 sub = `image.subs.len() - 1`
// （stg-harness/src/main.rs 建场处这样取，注释同步解释，不要在这份源码里插队声明新 sub）。
//
// follow-ups C13 摩擦逐条还账（本刀兑现，builder 版占位符全部退役）：
//   ① 环密度 `28 + global(RANK_SLOT) * 2`、环速度 `1.0fx + i as fx * 0.25fx`——两处都是
//      运行期表达式（`for` 循环变量 `i` 参与真实运算），不是 builder 期就地展开的字面量表。
//   ② `boss_set` 的 ratio 参数走 `$self_hp as fx / $self_hp_max as fx` 真定点除法
//      （DIVF），不是恒为 1.0fx 的占位符。
//   ③ 风铃摆 TURN 环走 `xformdef WIND_CHIME` 声明 + `fire(..., WIND_CHIME, ...)` 引用，
//      不是 16 发各自写一份 builder 期展开的 xform 序列。
// 附带修复一处 builder 版文档点名的摩擦：弹幕原点改用 `$self_x`/`$self_y`（boss 自身
// 位置，运行期读取）——boss 被 `patrol()` 巡游左右移动时弹幕原点跟着走，不再钉死在出生点。

const RANK_SLOT: int = 0; // 系统段 GVAR_RANK（stg_core::world::GVAR_RANK 的单一权威值）

xformdef WIND_CHIME {
    set_speed(2.0fx);
    @30 turn(90deg); // 30 帧后转 90°，风铃摆一记
}

async sub patrol() {
    loop {
        move_to(90, -120fx, 100fx, 2); // QuadOut 左
        wait(90);
        move_to(90, 120fx, 100fx, 2); // QuadOut 右
        wait(90);
    }
}

async sub timer_ui(spell: int) {
    loop {
        var t: int = 600;
        while t > 0 {
            var ratio: fx = $self_hp as fx / $self_hp_max as fx;
            boss_set(0, ratio, spell, t, 1, 1);
            wait(60);
            t = t - 60;
        }
    }
}

sub main() {
    spawn patrol();
    spawn timer_ui(1);

    var base: angle = 0deg;
    var volley: int = 0;

    loop {
        // C13①：环密度是真运行期表达式（rank 越高环越密），不是 builder 期字面量。
        var ways: int = 28 + global(RANK_SLOT) * 2;
        var step_i: int = 65536 / ways;
        var astep: angle = step_i as angle;

        for i in 0..5 {
            var appearance: int = i % 4; // 外观表只有 4 项（SMALL/MEDIUM/LARGE/STAR），循环
            var speed: fx = 1.0fx + i as fx * 0.25fx; // 环序越大越快
            _ = batch(appearance, $self_x, $self_y, ways, base, astep, 1, speed, 0fx);
        }

        // 隔轮追加一圈风铃摆 TURN 环（16-way，xformdef 引用）。
        if volley % 2 == 0 {
            for k in 0..16 {
                var ka: angle = (k * 4096) as angle;
                _ = fire(1, $self_x, $self_y, 0fx, ka, WIND_CHIME, none);
            }
        }

        base = base + 7deg; // 每轮基准角旋进，环层不重叠
        volley = volley + 1;
        wait(50);
    }
}
