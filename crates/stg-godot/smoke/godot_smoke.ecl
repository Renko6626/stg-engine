// Godot 桥 headless 全流程冒烟场景(task-4)——数据驱动:出一只敌+循环发弹/发通道B请求，
// 给 smoke.gd 一个"120 帧窗口里必有实体/必有请求/校验和会变"的最小可验证世界。
// task-6:B18 余量销账——enemy-owned 符卡可达面(A5:spawn_enemy 第 7 参),给 hud_spell 判别 +
// fields_info(符卡清弹 field)一条真实可达路径。清弹 field 只在符卡"结算"那一刻铺
// (settle_one_spell,spell.rs)——boss hp=8888 全程不受伤,hp_threshold=0 的 HP 路径永不
// 命中,故 time_limit 特意压到 8(远小于原型 3600)让符卡走超时路径在早期几帧内自然结算,
// smoke.gd 才能在一个短窗口内先摸到 active 期的 hud_spell、再摸到结算铺出的一帧 field。
// 槽位特意选 1(不是 0):main() 顶层已用 `boss_set(0, ...)` 手写 boss_ui 槽 0(task-4
// hud_boss 判别的既有前提),符卡 active 期/结算会自动覆写/清零绑定槽的 boss_ui——挂在
// 同一槽 0 会在这里的符卡早早结算时把那份手写值连锁清零,冲掉后面 task-4 的 hud_boss
// 断言(读的也是槽 0);两条 B18/B16② 判别面各占一槽,互不干扰。
// 颜色轴刀:弹型名/色名归内容包,不是引擎常量——单文件编译单元自带词表前奏
// (完整一份见 godot/ecl/demo/bullets.ecl)。
const BALL: int = 48;
const COLOR_CYAN: int = 8;

async sub smoke_spell_pattern() { loop { wait(60); } }
async sub smoke_boss() {
    spell_begin(1, 7, smoke_spell_pattern, 8, 50000, 0, 0);
    wait_spell();
}

sub main() {
    _ = spawn_enemy(0.0fx, -160.0fx, 9999, 0, 100, 0, none);
    _ = spawn_enemy(64.0fx, -120.0fx, 8888, 0, 0, 2, smoke_boss);
    // 第三只:**场内**(y=200,与自机同列 x=0)、hp 极高不死——给 frame_events 冒烟当靶子。
    // 必须在场内:自机弹越界回收线是 y ∈ [-64, 512],上面那两只在 y<-64 处,弹飞不到。
    // 它排在最后 → 敌池索引 2,不影响前面按索引 0 做的 mm oy == -160 判别。
    _ = spawn_enemy(0.0fx, 200.0fx, 9999, 0, 0, 0, none);
    // task-7:表现锚点 + 中段启动冒烟——bgm(3) 是 mark(9) 前最近一条 bgm 声明,正常流
    // 一跳跨过垫片(bgm 仍照直写生效=3);start=9 跳入则由 mark 自动补偿注入同一值 3。
    bgm(3);
    // task-4:B18 可达面补——bg/bg_phase/boss_set 三条此前从未被冒烟摸过。摆在 mark(9)
    // 之前(顶层线性位)使其同时进入 mark 的自动补偿扫描:bg_phase(1) 声明位置严格晚于
    // 本条 bg(2),补偿会注入;boss_set 不是 bgm/bg/bg_phase 三类锚点之一,不进补偿名单
    // ——中段启动分支(start=9)不会看到 boss_set 的效果,只有正常路径(start=0)会执行到它。
    bg(2);
    bg_phase(1);
    boss_set(0, 1.0fx, 7, 3600, 2, 1);
    mark(9);
    loop {
        _ = fire(BALL, COLOR_CYAN, 0.0fx, -160.0fx, 1.5fx, 0deg, none, none);
        emit_req(64, 1, 2, 3, 4, 5, 6);
        wait(30);
    }
}
