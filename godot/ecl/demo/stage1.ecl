// 杂兵段:三波×四机俯冲,瞄准三连发,底部退场(OOB 回收,任务随敌亡)。
async sub zako_dive() {
    move_to(90, $self_x, 140.0fx, 2);
    wait(90);
    for i in 0..3 {
        _ = fire(APPEARANCE_SMALL, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    move_to(150, $self_x, 560.0fx, 1);
    wait(600);
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
