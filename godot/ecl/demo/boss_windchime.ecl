// boss:非符(瞄准三叉)→ 风铃卡(rainbow.ecl 移植)。boss_main 是 enemy-owned 主任务
// (A5 乙案:spawn_enemy 第 7 参),敌死任务亡;boss_battle(STAGE 侧)用 enemy_hp 轮询等死。
//
// 发弹一律走**发射器**(shooter,syscall 6xx 族):配一遍 → 反复 `sh_fire`。发射器槽是
// **每任务 4 个**(`SHOOTERS_PER_TASK`),故 `boss_main` 与 `windchime_pattern` 各有自己的
// 0..3,互不干扰。裸 `fire`/`batch` 仍在(单发一次性的场合更短),本局只在没有复用价值处用。
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
    // 槽 0 = 米弹环。**`sh_ring` 自动均分整周**,不再手算 `65536 / ways`——手算那版
    // 在 ways 不整除 65536 时环合不拢(28/30/34 路各差 16~18 BAM ≈ 0.1°,只有 Hard 的
    // 32 路恰好整除);发射器逐颗算 `(i×65536)/n`、余数均摊,首尾精确闭合、相邻差极差 ≤1。
    sh_reset(0);
    sh_ring(0, 1);

    // 槽 1 = 风铃 BALL:整周 16 颗、初速 0、每颗挂 WIND_CHIME 变换段。
    // 配一遍即可——逐颗的 `k * 4096` 手算角度连同那个 16 次循环一起消失。
    sh_reset(1);
    sh_sprite(1, BALL, COLOR_CYAN);
    sh_speed(1, 0fx, 0fx);
    sh_count(1, 16, 1);
    sh_ring(1, 1);
    sh_xform(1, WIND_CHIME);

    var base: angle = 0deg;
    var volley: int = 0;
    loop {
        var ways: int = 28 + global(GVAR_RANK) * 2;
        sh_count(0, ways, 1);
        sh_angle(0, base, 0deg);   // 逐轮转 7deg;与 i 无关,故提到内层循环外
        for i in 0..5 {
            // 弹型钉死、颜色逐环轮转。RICE 是满色形,轮转全色安全;稀疏形
            // (HEART/BUTTERFLY 高 4 色是图集空格)不能这么轮。
            //
            // ⚠️ 这五环**不能**塌成一次 `sh_count(0, ways, 5)` + `sh_speed(0, 1.0fx, 0.25fx)`:
            // 速度层是发射器天生支持的(`n_speed`),但**颜色是发射器级的**、不逐层——
            // 塌了就得五环同色。故保留外层循环,每轮改 sprite+speed 再开火。
            var color: int = i % BULLET_COLOR_STRIDE;
            var speed: fx = 1.0fx + i as fx * 0.25fx;
            sh_sprite(0, RICE, color);
            sh_speed(0, speed, 0fx);
            sh_fire(0);
        }
        if volley % 2 == 0 {
            sh_fire(1);
        }
        base = base + 7deg;
        volley = volley + 1;
        wait(50);
    }
}

async sub boss_main() {
    spawn patrol();
    // 槽 0 = 非符的瞄准三叉。`sh_aim` 开 ⇒ `sh_angle` 的 angle0 是**相对自机方向**的偏移;
    // fan(ring 关)是**以基准方向为中心对称展开**,故 3 颗 × 12deg 步长 = 自机方向 −12/0/+12,
    // 与手写三行 `aim_player() ± 12deg` 逐位等价,但改颗数不必重算中心角。
    sh_reset(0);
    sh_sprite(0, OUTLINE, COLOR_PINK);
    sh_speed(0, 2.0fx, 0fx);
    sh_count(0, 3, 1);
    sh_angle(0, 0deg, 12deg);
    sh_aim(0, 1);

    // 非符段 10 秒:瞄准三叉,血条手喂(hp 真比值,C13② 同款)
    var t: int = 0;
    while t < 600 {
        boss_set(0, $self_hp as fx / $self_hp_max as fx, 0, 0, 2, 1);
        sh_fire(0);
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
    // 等 boss 死——**探活走 `enemy_alive`**(敌句柄打包刀之后敌号带 generation,槽复用可辨;
    // 探活读口刀之后 `enemy_alive` 是专用口,不再拿血量当探针——overkill 的敌 hp 是真实负值)。
    // 带 75 秒兜底:若敌 OOB 回收纪律不含敌类(敌界放宽,M0-13),超时退场路径下轮询会挂死。
    var t: int = 0;
    var waiting: int = 1;
    while waiting == 1 {
        if enemy_alive(boss) == 0 { waiting = 0; }
        if t > 4500 { waiting = 0; }
        wait(10);
        t = t + 10;
    }
    wait(60); // 死亡演出缓冲
}
