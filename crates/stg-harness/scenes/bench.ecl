// bench.ecl —— `stg-harness bench` 的三个**饱和档**脚本（follow-ups F5 还账，2026-08-01）
//
// ⚠️ 这不是一局游戏，别照着它学怎么写弹幕。**bench 量的是最烂情况的帧率，不是实际
// 凑巧的帧率**——每个入口只干一件事：把某一根轴打满，其余轴压到最低，好让那一行数字
// 只反映那根轴。要看"典型帧"请看现有的「全混合（真实规模）」那档。
//
// 三个入口（都不是 `main`；harness 走 `spawn_entry_named` 按名启动，见 main.rs
// `bench_ecl`）：
//   `bench_tasks`     ① 打满协程调度：256 个任务（TASK_CAP）每帧全部 resume，零发弹。
//   `bench_syscalls`  ② 打满 VM 派发：几十个任务，每个每帧烧一大把 `OP_SYS`。
//   `bench_shooter`   ③ 打满发射器开火路径：少量任务 × 接近上限的大环 `sh_fire`。
//
// `sub main()` 是**必须存在但不启动**的占位（表层语言要求每个编译单元恰有一个零参
// `main`）。harness 从不 `start_main` 它，故它一条指令都不会执行。
//
// ── ⚠️ 满负载要写 `wait(0)` 不是 `wait(1)`（写这份文件时踩的第一个坑）────────────
// 调度器的 wait 门禁是「`wait > 0` → 递减 1、跳过本帧」（`ecl::vm::run_tasks`），所以
// **`wait(1)` = 每隔一帧才跑一次**，`wait(0)` 才是"下一帧接着跑"。第一版三档全写的
// `wait(1)`，结果 `syscall 密` 那行出现了 mean(1023) < p50(2007) 的双峰——一半帧满载、
// 一半帧白送，而不是"每帧满载"。把这条写在这里，因为它在 ecl-lang.md 的 `wait` 节里
// 不显眼（那节讲的是 u16 截断），而它足以让一整档 bench 只测到一半负载。
//
// ── 预算红线（写这份文件时唯一真正的约束）────────────────────────────────────
// VM 有双层指令预算（`ecl::vm`）：**单任务 1024 条/帧** + **全局 65536 条/帧**，超限
// 是响亮的 `FAULT_BUDGET`（任务当场被杀 + `EVT_TASK_FAULT`，而 bench 自己只印时间、
// 不会报错）。故 main.rs 的 `bench_*` 那组测试逐档押运"任务数稳、零 fault、弹数没撞
// cap"。**改这份文件的迭代次数/任务数前先看那组测试**。

// ── 内容包词表（本文件用到的两行；完整一份见 godot/ecl/demo/bullets.ecl）──────
const RICE: int = 64; // 米弹——满色形，色轴随便取
const COLOR_RED: int = 0;

// `globals` 自由段（[16,1024)）的两个丢弃槽——② 把每帧算出来的和写进去，纯粹是给
// 累加值一个消费者，免得"算了不用"读起来像死代码（本编译器无 DCE，不写也照样发指令，
// 但写出来意图清楚）。
const BENCH_SINK_I: int = 16;
const BENCH_SINK_F: int = 17;

// ═══════════════════════════════════════════════════════════════════════════
// ① `bench_tasks` —— 打满任务池（TASK_CAP = 256）
// ═══════════════════════════════════════════════════════════════════════════
// 入口任务自己占 1 槽，`spawn` 出 255 个 `bench_drone`，正好把池填满。每个 drone 每帧
// 跑 `WAIT`+`JMP` 两条指令就让出——**每帧 256 个任务全部 resume**，调度器（存活位扫描 +
// copy-out/copy-back + 预算分账）被压满，而世界本体几乎无事可做（零弹零敌）。
// 这一档的数字≈纯调度价。
//
// ⚠️ 255 个 `spawn` 不能挤在一帧里：一帧一个任务只有 1024 条指令，255 次循环连
// SPAWN 带循环开销远超预算。故摊成 5 帧 × 51 个（每帧约 51×11≈560 条，留一半余量）。
// 预热 120 帧远长于这 5 帧，测量窗口里池早就是满的。
async sub bench_drone() {
    loop {
        wait(0);
    }
}

async sub bench_tasks() {
    for r in 0..5 {
        for i in 0..51 {
            spawn bench_drone();
        }
        wait(0);
    }
    loop {
        wait(0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ② `bench_syscalls` —— 打满 VM 派发（`OP_SYS`）
// ═══════════════════════════════════════════════════════════════════════════
// F5 点名要的那一档：让 `ecl::syscall::dispatch` 成为**可测比例**。任务数适中（60 个），
// 但每个任务每帧把预算几乎全烧在 syscall 上。
//
// **每帧的 syscall 条数是算得出来的**（报告里给的就是这个算法）：内层每轮 9 条——
//   `$frame` `$self_age` `$self_hp` `rand(101)` `global(GVAR_RANK)`  ← 5 条（int 轴）
//   `$self_x` `$self_y` `$player_x` `$player_y`                      ← 4 条（fx 轴）
// 外加每帧 2 条 `set_global`。故单任务 = 9 × BURN_ITERS + 2 = **254 条/帧**，
// 全场 60 个任务 = **15 240 条 syscall/帧**。号域覆盖 `0xx`（引擎变量）与 `1xx`
// （rand/global/set_global）两段，与号表重排刀那个外挂负载的号覆盖口径一致。
//
// `BURN_ITERS` / `BURN_TASKS` 的上限是**实测出来的**（两条预算各撞一次，二元一次方程
// 把"每帧多少条指令"这个原本看不见的量解了出来）：
//   · 单任务 1024 条：`BURN_ITERS` = 30 刚好过，31 时 60 个任务在第 1 帧齐刷刷 `FAULT_BUDGET`。
//   · 全局 65536 条：`BURN_TASKS` = 69 刚好过，70 时**恰好 1 个**任务 fault（71→2 个、
//     74→5 个——共享余额按池索引升序消耗，撞破后从尾巴开始逐个饿死，正是 I4 的样子）。
// 解出**单任务 ≈ 942 条指令/帧**（28 轮 × 每轮 ≈ 33 条 + 每帧固定 ≈ 18 条），其中 254 条
// 是 syscall ⇒ **派发密度约 27%**。取 `BURN_ITERS = 28`（上限 30 的 93%）、
// `BURN_TASKS = 60`（上限 69 的 87%），两头都留余量给将来 codegen 每轮多发一两条指令的
// 情况——真多了也不会静默，`bench_syscalls_holds_all_burner_tasks` 会红。
//
// 累加器每帧清零：`ia` 每帧至多攒到几万、`fa` 至多几百 px——**debug 构建下定点加法
// 溢出是 panic**，不清零的话跑长了必炸（`--frames 100000` 之类）。
const BURN_ITERS: int = 28;
const BURN_TASKS: int = 60;

async sub bench_sys_burn() {
    var ia: int = 0;
    var fa: fx = 0fx;
    loop {
        ia = 0;
        fa = 0fx;
        for i in 0..BURN_ITERS {
            ia = ia + $frame + $self_age + $self_hp + rand(101) + global(GVAR_RANK);
            fa = fa + $self_x + $self_y + $player_x + $player_y;
        }
        set_global(BENCH_SINK_I, ia);
        set_global(BENCH_SINK_F, fa as int);
        wait(0);
    }
}

async sub bench_syscalls() {
    for i in 0..BURN_TASKS {
        spawn bench_sys_burn();
    }
    loop {
        wait(0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ③ `bench_shooter` —— 打满发射器开火路径（`sh_fire` 网格循环 + 逐颗 create_bullet）
// ═══════════════════════════════════════════════════════════════════════════
// 8 个任务，每个配一个 `n_angle` 打到 **255**（`sh_count` 双边钳 `[0,255]`，写 300 得到的
// 是 255 而不是报错）的整周大环，按固定节奏开火。
//
// ⚠️ **不挂 `sh_task`**：那是"每颗弹派一个任务"，一句 `sh_fire` 就吃 255 个任务槽（池共
// 256），瞬间打爆——这一档会变成任务池测试。同理不挂 `sh_xform`（一句吃 255 个段，段池
// 共 2048）。这一档要的是干净的"网格循环 + create_bullet"。
//
// 稳态账（保证不撞弹池 8192 上限，撞了 `sh_fire` 会短路、反而少干活）：
//   8 个发射器 × 255 颗 ÷ 10 帧 = 204 颗/帧；速度 10px/帧、从场内散点出发，
//   到最远边界（半宽 192 + 越界边距 64 = 256px；纵向 224+64=288px）约 26~29 帧
//   ⇒ 稳态 ≈ 204 × 28 ≈ 5700 颗，留 30% 余量。实测稳态 ~5300 颗，印在 bench 那张表里，
//   并由 `bench_shooter_stays_under_bullet_cap` 押住上沿。
//   （`SHOOTER_PERIOD = 9` 而不是 10：`wait(n)` 是"跳过 n 帧"，实际周期 = n+1。）
//
// `add_lives(255)`：这一档满屏弹会持续打死自机（残机默认 3，决死 8 帧 + 重生无敌 120 帧
// ⇒ 约 400 帧就 GAMEOVER）。GAMEOVER 之后自机不再参与移动/发弹，**测量窗口中途换了
// 世界的行为**——那是最该避免的"不稳态"。顶满 255 条命（钳位，不回绕）后 720 帧内绝无
// 可能耗尽，全程停在同一个"死→重生→再死"的循环里。
const SHOOTER_TASKS: int = 8;
const SHOOTER_WAYS: int = 255;
const SHOOTER_PERIOD: int = 9;

async sub bench_ring(x: fx, y: fx, phase: int) {
    // 相位错开：8 个发射器不要在同一帧一起开火（否则每 10 帧一根 2040 颗的尖刺，
    // p99 变成"撞见没撞见那一帧"的抽签，读不出东西）。摊开成每帧恰好一个开火。
    wait(phase);
    sh_reset(0);
    sh_sprite(0, RICE, COLOR_RED);
    sh_offset_abs(0, x, y);
    sh_ring(0, 1); // 整周环：n_angle 颗自动均分，余数均摊、首尾精确闭合
    sh_count(0, SHOOTER_WAYS, 1);
    sh_speed(0, 10.0fx, 0fx);
    loop {
        sh_angle(0, ($self_age * 137) as angle, 0deg); // 逐轮旋进，环层不重叠
        sh_fire(0);
        wait(SHOOTER_PERIOD);
    }
}

async sub bench_shooter() {
    add_lives(255); // 见上：防测量窗口中途 GAMEOVER
    for i in 0..SHOOTER_TASKS {
        // 出弹点摊在场内一条横线上（y=160），x 从 -168 到 +168 等距 48px。
        spawn bench_ring((i * 48 - 168) as fx, 160.0fx, i);
    }
    loop {
        wait(0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 占位 main（表层语言要求恰有一个；harness 永不启动它）
// ═══════════════════════════════════════════════════════════════════════════
sub main() {
    loop {
        wait(0);
    }
}
