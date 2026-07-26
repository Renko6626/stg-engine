// boss:非符(瞄准三叉)→ 风铃卡(rainbow.ecl 移植)。boss_main 是 enemy-owned 主任务
// (A5 乙案:spawn_enemy 第 7 参),敌死任务亡;boss_battle(STAGE 侧)用 enemy_hp 轮询等死。
const SPELL_WINDCHIME: int = 1;

xformdef WIND_CHIME {
    set_speed(2.0fx);
    @30 turn(90deg);
}

async sub patrol() {
    loop {
        move_to(90, -120fx, 100fx, 2);
        wait(90);
        move_to(90, 120fx, 100fx, 2);
        wait(90);
    }
}

async sub windchime_pattern() {
    var base: angle = 0deg;
    var volley: int = 0;
    loop {
        var ways: int = 28 + global(GVAR_RANK) * 2;
        var step_i: int = 65536 / ways;
        var astep: angle = step_i as angle;
        for i in 0..5 {
            // 弹型钉死、颜色逐环轮转。BULLET_RICE 是满色形,轮转全色安全;稀疏形
            // (HEART/BUTTERFLY 高 4 色是图集空格)不能这么轮。
            var color: int = i % BULLET_COLOR_STRIDE;
            var speed: fx = 1.0fx + i as fx * 0.25fx;
            _ = batch(BULLET_RICE, color, $self_x, $self_y, ways, base, astep, 1, speed, 0fx);
        }
        if volley % 2 == 0 {
            for k in 0..16 {
                var ka: angle = (k * 4096) as angle;
                _ = fire(BULLET_BALL_M, COLOR_CYAN, $self_x, $self_y, 0fx, ka, WIND_CHIME, none);
            }
        }
        base = base + 7deg;
        volley = volley + 1;
        wait(50);
    }
}

async sub boss_main() {
    spawn patrol();
    // 非符段 10 秒:瞄准三叉,血条手喂(hp 真比值,C13② 同款)
    var t: int = 0;
    while t < 600 {
        boss_set(0, $self_hp as fx / $self_hp_max as fx, 0, 0, 2, 1);
        _ = fire(BULLET_BALL_S, COLOR_ROSE, $self_x, $self_y, 2.0fx, aim_player(), none, none);
        _ = fire(BULLET_BALL_S, COLOR_ROSE, $self_x, $self_y, 2.0fx, aim_player() + 12deg, none, none);
        _ = fire(BULLET_BALL_S, COLOR_ROSE, $self_x, $self_y, 2.0fx, aim_player() - 12deg, none, none);
        wait(20);
        t = t + 20;
    }
    spell_begin(0, SPELL_WINDCHIME, windchime_pattern, 3600, 100000, 0, 0);
    wait_spell();
    // 超时未破:退场(顶部飞出,OOB 回收 → boss_battle 的轮询放行;被击破则本任务已随敌亡)
    move_to(120, 0fx, -600.0fx, 1);
    wait(600);
}

sub boss_battle() {
    // hp=900(用户裁定,demo 平衡):tier0 三发/轮实测≈44dps、站桩~20s 可破卡,「取得」路径
    // 人工可达(实测值,测法=静止 boss/task none/自机站桩 SHOT,终审仓外探针实测)——原 2600
    // 在非符 10 秒(自机同期几乎不可能追上耗时)+符卡阶段几乎打不穿。
    var boss: int = spawn_enemy(0.0fx, 96.0fx, 900, 1, 5000, 1, boss_main);
    // 等 boss 死(enemy_hp<0)——带 75 秒兜底:若敌 OOB 回收纪律不含敌类(敌界放宽,
    // M0-13),超时退场路径下轮询会挂死,兜底保 demo 流程必然推进。
    var t: int = 0;
    var waiting: int = 1;
    while waiting == 1 {
        if enemy_hp(boss) < 0 { waiting = 0; }
        if t > 4500 { waiting = 0; }
        wait(10);
        t = t + 10;
    }
    wait(60); // 死亡演出缓冲
}
