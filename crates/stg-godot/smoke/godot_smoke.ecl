// Godot 桥 headless 全流程冒烟场景(task-4)——数据驱动:出一只敌+循环发弹/发通道B请求，
// 给 smoke.gd 一个"120 帧窗口里必有实体/必有请求/校验和会变"的最小可验证世界。
sub main() {
    _ = spawn_enemy(0.0fx, -160.0fx, 9999, 0, 100);
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
        _ = fire(1, 0.0fx, -160.0fx, 1.5fx, 0deg, none, none);
        emit_req(64, 1, 2, 3, 4, 5, 6);
        wait(30);
    }
}
