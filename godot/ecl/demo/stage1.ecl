// 杂兵段:三波×四机俯冲,瞄准三连发,退场靠 D9(敌主协程返回即自燃——ZUN ECL 语义,
// `ecl::vm::run_tasks` 的 Exec::End 分支,命中 main_task 即标 ENEMY_DYING,相位 9 回收):
// 退场目标 y=500 仍在回收线内(FIELD_HEIGHT(448)+ENEMY_OOB_MARGIN(256)=704),不靠越界
// 兜底——move_to 150 帧移动完毕、wait(150) 等它走完后本 sub 自然 return,敌随之静默退场。
async sub zako_dive() {
    move_to(90, $self_x, 140.0fx, 2);
    wait(90);
    for i in 0..3 {
        _ = fire(OUTLINE, COLOR_DARK_CYAN, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    move_to(150, $self_x, 500.0fx, 1);
    wait(150);
}

sub stage1() {
    for w in 0..3 {
        for i in 0..4 {
            _ = spawn_enemy((i * 96 - 144) as fx, 2.0fx, 40, 1, 300, 0, zako_dive);
            wait(15);
        }
        wait(150);
    }
    wait(120); // 清场缓冲
}
