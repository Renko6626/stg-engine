// 彩虹风铃符卡（M1.9 T4 —— .ecl 表层语言重写，金向量二号 scene 2 的脚本源）
//
// 结构照 docs/superpowers/specs/2026-07-18-m19-ecl-language.md 语言草图：main 五环 loop +
// patrol/timer_ui 两个 async 子任务。声明序不再决定 SubId——编译器按名称字典序规范排序
// （named entry ABI，2026-07-20），建场处走 `resolve_entry("patrol")` 等名字查询而非
// 位置索引。
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
//
// follow-ups C14 去魔数（本刀兑现）：风铃摆 TURN 环固定发一种外观，`fire(1, ...)` 换成
// 注入引擎常量 `fire(APPEARANCE_MEDIUM, ...)`——纯改书写不改值（折叠回同一字面量 1），
// 金向量校验和不变。`APPEARANCE_SMALL/MEDIUM/LARGE/STAR`/`GVAR_RANK`/`GLOBALS_SYS_SEGMENT`
// 现由 stg-core `engine_consts!` 注册表统一注入 `.ecl` 命名空间（见 docs/ecl-lang.md
// "引擎常量"节）；`for i in 0..5` 里的 `i % 4` 是有意轮转全表，非单指某个 appearance，
// 不换。下面 `RANK_SLOT` 仍手写镜像 `GVAR_RANK`（本刀未动，留 follow-ups 记账）。

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
                _ = fire(APPEARANCE_MEDIUM, $self_x, $self_y, 0fx, ka, WIND_CHIME, none);
            }
        }

        base = base + 7deg; // 每轮基准角旋进，环层不重叠
        volley = volley + 1;
        wait(50);
    }
}
