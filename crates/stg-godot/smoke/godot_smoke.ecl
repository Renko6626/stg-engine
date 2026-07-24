// Godot 桥 headless 全流程冒烟场景(task-4)——数据驱动:出一只敌+循环发弹/发通道B请求，
// 给 smoke.gd 一个"120 帧窗口里必有实体/必有请求/校验和会变"的最小可验证世界。
sub main() {
    _ = spawn_enemy(0.0fx, -160.0fx, 9999, 0, 100);
    loop {
        _ = fire(1, 0.0fx, -160.0fx, 1.5fx, 0deg, none, none);
        emit_req(64, 1, 2, 3, 4, 5, 6);
        wait(30);
    }
}
